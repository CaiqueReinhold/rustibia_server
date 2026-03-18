use tokio::sync::mpsc;

mod connection;
mod session;
mod world;

pub use connection::{ConnectionActor, ConnectionCommand};
pub use session::SessionActor;
pub use world::{BroadcastMessage, Tick, WorldActor, WorldCommand};

#[derive(Clone, Debug)]
pub struct ActorHandle<T: Clone> {
    tx: mpsc::Sender<T>,
}

impl<T: Clone> ActorHandle<T> {
    pub async fn send(&self, cmd: T) -> Result<(), mpsc::error::SendError<T>> {
        self.tx.send(cmd).await?;
        Ok(())
    }
}
