use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{error, info};

use super::{SharedContext, connection::ConnectionError, session::SessionActor};
use crate::{actors::connection::ConnectionActorHandle, game::events::BroadcastMessage};
use crate::{
    config::CONFIG,
    entities::agent::Agent,
    messages::{ClientMessage, ServerMessage},
    persistence::{
        auth::{AccountId, AuthRepository},
        player::PlayerRepository,
    },
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
    world_ctx: SharedContext,
    player_repo: Arc<PlayerRepository>,
    auth_repo: Arc<AuthRepository>,
    brx: broadcast::Receiver<BroadcastMessage>,
}

impl AuthActor {
    pub fn start(
        session_id: String,
        conn_rx: oneshot::Receiver<ConnectionActorHandle>,
        player_repo: Arc<PlayerRepository>,
        auth_repo: Arc<AuthRepository>,
        brx: broadcast::Receiver<BroadcastMessage>,
        world_ctx: SharedContext,
    ) -> AuthActorHandle {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);

        tokio::spawn(async move {
            let actor = Self {
                session_id,
                rx,
                world_ctx,
                player_repo,
                auth_repo,
                brx,
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

        let (character_id, auth_token) = match msg {
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

        let account_id: AccountId = match self.auth_repo.validate_token(&auth_token).await {
            Ok(id) => id,
            Err(e) => {
                info!(session = self.session_id, "Token validation failed: {e}");
                let _ = connection.send_message(ServerMessage::LoginError).await;
                return Err(e.into());
            }
        };

        let player = match self
            .player_repo
            .get_by_id_for_account(character_id, account_id)
            .await
        {
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
            self.world_ctx.world.clone(),
            self.brx.resubscribe(),
            self.world_ctx.shared_map.clone(),
            self.world_ctx.persistence.clone(),
        );

        connection.set_session(session).await?;

        info!(
            session = self.session_id,
            "Auth complete, session handed off"
        );
        Ok(())
    }
}
