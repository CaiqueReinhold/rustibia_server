use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::CONFIG;
use crate::persistence::online::OnlineRepository;
use crate::persistence::player::{PlayerRepository, PlayerSnapshot};

#[derive(Clone, Debug)]
pub enum PersistenceCommand {
    SavePlayer(PlayerSnapshot),
    MarkOnline(u32),
    MarkOffline(u32),
}

#[derive(Clone, Debug)]
pub struct PersistenceActorHandle {
    tx: mpsc::Sender<PersistenceCommand>,
}

impl PersistenceActorHandle {
    pub async fn save_player(
        &self,
        player: PlayerSnapshot,
    ) -> Result<(), mpsc::error::SendError<PersistenceCommand>> {
        self.tx.send(PersistenceCommand::SavePlayer(player)).await?;
        Ok(())
    }

    /// Non-blocking, synchronous — callable from `Drop`, which cannot `.await`.
    ///
    /// Uses `try_send`, so a full channel drops the update rather than blocking the
    /// game loop. This is presentational data; a log line is the right response.
    pub fn mark_online(&self, character_id: u32) {
        if self
            .tx
            .try_send(PersistenceCommand::MarkOnline(character_id))
            .is_err()
        {
            tracing::warn!(
                character_id,
                "dropped online marker: persistence channel full"
            );
        }
    }

    pub fn mark_offline(&self, character_id: u32) {
        if self
            .tx
            .try_send(PersistenceCommand::MarkOffline(character_id))
            .is_err()
        {
            tracing::warn!(
                character_id,
                "dropped offline marker: persistence channel full"
            );
        }
    }

    /// A handle with no actor behind it, plus the receiving end so a test can assert
    /// what was sent. Exists because `OnlineRegistry` now requires a handle, and
    /// spinning up a real `PersistenceActor` would drag a database into what are
    /// otherwise pure in-memory tests.
    #[cfg(test)]
    pub fn for_test(buffer: usize) -> (Self, mpsc::Receiver<PersistenceCommand>) {
        let (tx, rx) = mpsc::channel(buffer);
        (PersistenceActorHandle { tx }, rx)
    }
}

pub struct PersistenceActor {
    rx: mpsc::Receiver<PersistenceCommand>,
    repo: Arc<PlayerRepository>,
    online: Arc<OnlineRepository>,
}

impl PersistenceActor {
    pub fn start(
        repo: Arc<PlayerRepository>,
        online: Arc<OnlineRepository>,
    ) -> PersistenceActorHandle {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        tokio::spawn(async move {
            let actor = Self { rx, repo, online };
            actor.run().await;
        });
        PersistenceActorHandle { tx }
    }

    async fn run(mut self) {
        info!("Persistence actor started");
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                PersistenceCommand::SavePlayer(snapshot) => {
                    let player_id = snapshot.id;
                    if let Err(e) = self.repo.save(&snapshot).await {
                        error!(player_id, "Failed to save player: {e}");
                    }
                }
                PersistenceCommand::MarkOnline(character_id) => {
                    if let Err(e) = self.online.mark_online(character_id).await {
                        error!(character_id, "Failed to mark player online: {e}");
                    }
                }
                PersistenceCommand::MarkOffline(character_id) => {
                    if let Err(e) = self.online.mark_offline(character_id).await {
                        error!(character_id, "Failed to mark player offline: {e}");
                    }
                }
            }
        }
        info!("Persistence actor stopped");
    }
}
