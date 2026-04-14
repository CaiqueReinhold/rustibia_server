//! The connection actor is responsible for framing and message
//! serialization/deserialization. It owns the raw [`TcpStream`] and translates
//! between bytes on the wire and typed commands.
//!
//! Incoming client messages are routed to whichever upstream actor is currently
//! active. On connection start the upstream is [`AuthActor`](super::auth::AuthActor);
//! once authentication completes a [`ConnectionCommand::SetSession`] switches it
//! to the [`SessionActor`](super::session::SessionActor) for the remainder of the
//! connection's lifetime.

use anyhow::Result;
use futures::sink::SinkExt;
use thiserror::Error;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::select;
use tokio::{net::TcpStream, sync::mpsc};
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, FramedWrite};
use tracing::{debug, error, info};

use super::{auth::AuthCommand, session::SessionCommand, ActorHandle};
use crate::config::CONFIG;
use crate::messages::{ClientMessage, GameMessageCodec, MessageDecodeError, ServerMessage};

#[derive(Clone, Debug)]
pub enum ConnectionCommand {
    Close,
    SendPlayerMessage(ServerMessage),
    SetSession(ActorHandle<SessionCommand>),
}

#[derive(Error, Debug)]
pub enum ConnectionError {
    #[error("Read error")]
    ReadError(#[from] MessageDecodeError),
    #[error("Server error")]
    ServerError,
    #[error("Connnection is closed")]
    ConnectionClosed,
    #[error("Wrong message type")]
    WrongMessageType,
}

enum Upstream {
    Auth(ActorHandle<AuthCommand>),
    Session(ActorHandle<SessionCommand>),
}

pub struct ConnectionActor {
    session_id: String,
    rx: mpsc::Receiver<ConnectionCommand>,
    reader: FramedRead<OwnedReadHalf, GameMessageCodec>,
    writer: FramedWrite<OwnedWriteHalf, GameMessageCodec>,
    upstream: Upstream,
}

impl ConnectionActor {
    pub fn start(
        session_id: String,
        stream: TcpStream,
        auth: ActorHandle<AuthCommand>,
    ) -> ActorHandle<ConnectionCommand> {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        let (read, write) = stream.into_split();

        let actor = Self {
            session_id,
            rx,
            reader: FramedRead::new(read, GameMessageCodec {}),
            writer: FramedWrite::new(write, GameMessageCodec {}),
            upstream: Upstream::Auth(auth),
        };

        tokio::spawn(actor.run());

        ActorHandle { tx }
    }

    async fn run(mut self) {
        info!(session = self.session_id, "Connection started");
        loop {
            debug!(session = self.session_id, "Connection waiting for messages");
            let result = select! {
                msg = self.reader.next() => self.handle_client_message(msg).await,
                cmd = self.rx.recv() => self.handle_connection_command(cmd).await
            };
            if let Err(e) = result {
                error!(session = self.session_id, "Connection error: {e}");
                break;
            }
        }
        if let Upstream::Session(session) = self.upstream {
            let _ = session.send(SessionCommand::Close).await;
        }
        info!(session = self.session_id, "Connection finished");
    }

    async fn handle_client_message(
        &self,
        message: Option<Result<ClientMessage, MessageDecodeError>>,
    ) -> Result<(), ConnectionError> {
        let msg = message.ok_or(ConnectionError::ConnectionClosed)??;
        debug!(
            session = self.session_id,
            "Connection received message: {:?}", msg
        );
        let send_result = match &self.upstream {
            Upstream::Auth(auth) => auth
                .send(AuthCommand::ReceivePlayerMessage(msg))
                .await
                .is_err(),
            Upstream::Session(session) => session
                .send(SessionCommand::ReceivePlayerMessage(msg))
                .await
                .is_err(),
        };
        if send_result {
            return Err(ConnectionError::ServerError);
        }
        Ok(())
    }

    async fn handle_connection_command(
        &mut self,
        command: Option<ConnectionCommand>,
    ) -> Result<(), ConnectionError> {
        let cmd = command.ok_or(ConnectionError::ConnectionClosed)?;
        match cmd {
            ConnectionCommand::Close => {
                return Err(ConnectionError::ConnectionClosed);
            }
            ConnectionCommand::SetSession(session) => {
                info!(session = self.session_id, "Upstream switched to session");
                self.upstream = Upstream::Session(session);
            }
            ConnectionCommand::SendPlayerMessage(msg) => {
                info!(session = self.session_id, "Sending player msg: {:?}", msg);
                if self.writer.send(msg).await.is_err() {
                    return Err(ConnectionError::ServerError);
                }
            }
        }
        Ok(())
    }
}
