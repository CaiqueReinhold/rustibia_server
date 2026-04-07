use std::net::SocketAddr;
use tracing::{error, info, warn};

use anyhow::Result;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot,
};
use uuid::{NoContext, Timestamp};

use crate::{
    actors::{
        connection::{ConnectionActor, ConnectionCommand},
        session::SessionActor,
    },
    Context,
};

pub struct Listener {
    inner: tokio::net::TcpListener,
}

impl Listener {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let inner = TcpListener::bind(addr).await?;
        Ok(Self { inner })
    }

    pub async fn listen(&self, context: Context) {
        loop {
            match self.inner.accept().await {
                Ok((stream, addr)) => {
                    info!("new connection from {:?}", addr);
                    if let Err(e) = Self::accept_connection(stream, &context).await {
                        error!("accept_connection failed: {e}")
                    }
                }
                Err(e) => {
                    warn!("Failed to accept connection: {e}");
                }
            }
        }
    }

    async fn accept_connection(stream: TcpStream, context: &Context) -> Result<()> {
        let sesion_id = uuid::Uuid::new_v7(Timestamp::now(NoContext)).to_string();
        let (conn_tx, conn_rx) = oneshot::channel();
        let session = SessionActor::start(
            sesion_id.clone(),
            conn_rx,
            context.world.clone(),
            context.player_repo.clone(),
            context.broadcast_receiver.resubscribe(),
            context.shared_map.clone(),
        );
        let connection = ConnectionActor::start(sesion_id, stream, session);
        if let Err(conn) = conn_tx.send(connection) {
            info!("failed to open connection");
            conn.send(ConnectionCommand::Close).await?;
        }
        Ok(())
    }
}
