use tokio::sync::mpsc;

pub mod auth;
pub mod connection;
mod item_action;
mod map_query;
mod player_query;
pub mod session;
pub mod world;

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
