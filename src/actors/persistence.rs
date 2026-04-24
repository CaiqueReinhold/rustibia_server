use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{error, info};

use crate::config::CONFIG;
use crate::persistence::player::{PlayerRepository, PlayerSnapshot};

#[derive(Clone)]
pub enum PersistenceCommand {
    SavePlayer(PlayerSnapshot),
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
}

pub struct PersistenceActor {
    rx: mpsc::Receiver<PersistenceCommand>,
    repo: Arc<PlayerRepository>,
}

impl PersistenceActor {
    pub fn start(repo: Arc<PlayerRepository>) -> PersistenceActorHandle {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);
        tokio::spawn(async move {
            let actor = Self { rx, repo };
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
            }
        }
        info!("Persistence actor stopped");
    }
}
