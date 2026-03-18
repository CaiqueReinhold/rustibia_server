//! The connection actor is responsible for framing and message
//! serialization/deserialization. It owns the raw [`TcpStream`] and translates
//! between bytes on the wire and typed commands exchanged with the
//! [`SessionActor`](crate::actors::SessionActor).

use anyhow::Result;
use futures::sink::SinkExt;
use thiserror::Error;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::select;
use tokio::{net::TcpStream, sync::mpsc};
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, FramedWrite};
use tracing::{debug, error, info};

use super::{session::SessionCommand, ActorHandle};
use crate::config::CONFIG;
use crate::messages::{ClientMessage, GameMessageCodec, MessageDecodeError, ServerMessage};

#[derive(Clone, Debug)]
pub enum ConnectionCommand {
    Close,
    AuthOk,
    SendPlayerMessage(ServerMessage),
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
    #[error("Invalid authentication")]
    InvalidAuth,
}

pub struct ConnectionActor {
    session_id: String,
    rx: mpsc::Receiver<ConnectionCommand>,
    reader: FramedRead<OwnedReadHalf, GameMessageCodec>,
    writer: FramedWrite<OwnedWriteHalf, GameMessageCodec>,
    session: ActorHandle<SessionCommand>,
}

impl ConnectionActor {
    pub fn start(
        session_id: String,
        stream: TcpStream,
        session: ActorHandle<SessionCommand>,
    ) -> ActorHandle<ConnectionCommand> {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        let (read, write) = stream.into_split();

        let actor = Self {
            session_id,
            rx,
            reader: FramedRead::new(read, GameMessageCodec {}),
            writer: FramedWrite::new(write, GameMessageCodec {}),
            session,
        };

        tokio::spawn(actor.run());

        ActorHandle { tx }
    }

    async fn run(mut self) {
        info!(session = self.session_id, "Connection started");
        if self.authenticate().await.is_err() {
            info!(session = self.session_id, "Auth failed");
            return;
        }
        info!(session = self.session_id, "Connection authenticated");
        loop {
            debug!(session = self.session_id, "Connection waiting for messages");
            let result = select! {
                cmd = self.reader.next() => self.handle_client_command(cmd).await,
                cmd = self.rx.recv() => self.handle_connection_command(cmd).await
            };
            if let Err(e) = result {
                error!(session = self.session_id, "Connection error: {e}");
                break;
            }
        }
        info!(session = self.session_id, "Connection finished");
    }

    async fn handle_client_command(
        &self,
        message: Option<Result<ClientMessage, MessageDecodeError>>,
    ) -> Result<(), ConnectionError> {
        let msg = message.ok_or(ConnectionError::ConnectionClosed)??;
        debug!(
            session = self.session_id,
            "Connection received message: {:?}", msg
        );
        if self
            .session
            .send(SessionCommand::ReceivePlayerMessage(msg))
            .await
            .is_err()
        {
            return Err(ConnectionError::ServerError);
        };
        Ok(())
    }

    async fn handle_connection_command(
        &mut self,
        command: Option<ConnectionCommand>,
    ) -> Result<(), ConnectionError> {
        let cmd = command.ok_or(ConnectionError::ConnectionClosed)?;
        info!(
            session = self.session_id,
            "Connection received command: {:?}", cmd
        );
        match cmd {
            ConnectionCommand::Close => {
                return Err(ConnectionError::ConnectionClosed);
            }
            ConnectionCommand::AuthOk => {
                return Err(ConnectionError::WrongMessageType);
            }
            ConnectionCommand::SendPlayerMessage(msg) => {
                if self.writer.send(msg).await.is_err() {
                    return Err(ConnectionError::ServerError);
                }
            }
        }
        Ok(())
    }

    async fn authenticate(&mut self) -> Result<()> {
        let msg = self
            .reader
            .next()
            .await
            .ok_or(ConnectionError::ConnectionClosed)??;
        if let ClientMessage::Login {
            character_id,
            auth_token,
        } = msg
        {
            self.session
                .send(SessionCommand::Login {
                    character_id,
                    auth_token,
                })
                .await?;
            let cmd = self.rx.recv().await.ok_or(ConnectionError::ServerError)?;
            if let ConnectionCommand::AuthOk = cmd {
                return Ok(());
            }

            Err(ConnectionError::InvalidAuth.into())
        } else {
            Err(ConnectionError::WrongMessageType.into())
        }
    }
}
