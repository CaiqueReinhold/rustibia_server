use std::sync::Arc;

use anyhow::Result;
use arc_swap::ArcSwap;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{error, info};

use super::{connection::ConnectionError, session::SessionActor};
use crate::{
    actors::{
        connection::ConnectionActorHandle, persistence::PersistenceActorHandle,
        world::WorldActorHandle,
    },
    game::events::BroadcastMessage,
};
use crate::{
    config::CONFIG,
    entities::{agent::Agent, map::GameMap},
    messages::{ClientMessage, ServerMessage},
    persistence::player::PlayerRepository,
};

#[derive(Clone, Debug)]
pub enum AuthCommand {
    ReceivePlayerMessage(ClientMessage),
}

#[derive(Clone, Debug)]
pub struct AuthActorHandle {
    tx: mpsc::Sender<AuthCommand>,
}

impl AuthActorHandle {
    pub async fn receive_message(
        &self,
        msg: ClientMessage,
    ) -> Result<(), mpsc::error::SendError<AuthCommand>> {
        self.tx.send(AuthCommand::ReceivePlayerMessage(msg)).await?;
        Ok(())
    }
}

pub struct AuthActor {
    session_id: String,
    rx: mpsc::Receiver<AuthCommand>,
    world: WorldActorHandle,
    player_repo: Arc<PlayerRepository>,
    brx: broadcast::Receiver<BroadcastMessage>,
    shared_map: Arc<ArcSwap<GameMap>>,
    persistence: PersistenceActorHandle,
}

impl AuthActor {
    pub fn start(
        session_id: String,
        conn_rx: oneshot::Receiver<ConnectionActorHandle>,
        world: WorldActorHandle,
        player_repo: Arc<PlayerRepository>,
        brx: broadcast::Receiver<BroadcastMessage>,
        shared_map: Arc<ArcSwap<GameMap>>,
        persistence: PersistenceActorHandle,
    ) -> AuthActorHandle {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);

        tokio::spawn(async move {
            let actor = Self {
                session_id,
                rx,
                world,
                player_repo,
                brx,
                shared_map,
                persistence,
            };
            actor.run(conn_rx).await;
        });

        AuthActorHandle { tx }
    }

    async fn run(mut self, conn_rx: oneshot::Receiver<ConnectionActorHandle>) {
        info!(session = self.session_id, "Auth actor started");

        let connection = match conn_rx.await {
            Ok(c) => c,
            Err(_) => return,
        };

        if let Err(e) = self.authenticate(&connection).await {
            info!(session = self.session_id, "Auth failed: {e}");
            let _ = connection.close().await;
        }
    }

    async fn authenticate(&mut self, connection: &ConnectionActorHandle) -> Result<()> {
        let msg = match self.rx.recv().await {
            Some(AuthCommand::ReceivePlayerMessage(msg)) => msg,
            None => return Err(ConnectionError::ConnectionClosed.into()),
        };

        let (character_id, _auth_token) = match msg {
            ClientMessage::Login {
                character_id,
                auth_token,
            } => (character_id, auth_token),
            msg => {
                info!("{:?}", msg);
                let _ = connection.send_message(ServerMessage::LoginError).await;
                return Err(ConnectionError::WrongMessageType.into());
            }
        };

        let player = match self.player_repo.get_by_id(character_id).await {
            Ok(p) => p,
            Err(e) => {
                error!(session = self.session_id, "Player lookup failed: {e}");
                let _ = connection.send_message(ServerMessage::LoginError).await;
                return Err(e.into());
            }
        };

        let session = SessionActor::start(
            self.session_id.clone(),
            connection.clone(),
            Agent::from_player(player),
            self.world.clone(),
            self.brx.resubscribe(),
            self.shared_map.clone(),
            self.persistence.clone(),
        );

        connection.set_session(session).await?;

        info!(
            session = self.session_id,
            "Auth complete, session handed off"
        );
        Ok(())
    }
}
