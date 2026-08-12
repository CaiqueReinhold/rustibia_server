use std::collections::HashMap;

use tokio::sync::mpsc;
use tracing::info;

use crate::{
    actors::message_router::MessageRouterActorHandle,
    config::CONFIG,
    entities::{
        agent::AgentKey,
        chat::{Channel, ChannelId},
    },
    game::game_config::GAME_CONFIG,
};

#[derive(Debug)]
pub enum ChatCommand {
    SendPrivateMessage {
        sender: AgentKey,
        receiver: AgentKey,
        message: String,
    },
    SendChannelMessage {
        player: AgentKey,
        channel: ChannelId,
        message: String,
    },
    JoinChannel {
        player: AgentKey,
        channel: ChannelId,
    },
    LeaveChannel {
        player: AgentKey,
        channel: ChannelId,
    },
}

#[derive(Clone, Debug)]
pub struct ChatActorHandle {
    tx: mpsc::Sender<ChatCommand>,
    available_channels: Box<[(ChannelId, String)]>,
}

impl ChatActorHandle {
    #[cfg(test)]
    pub fn for_test() -> (Self, mpsc::Receiver<ChatCommand>) {
        let (tx, rx) = mpsc::channel(64);
        (
            Self {
                tx,
                available_channels: Box::new([(1, "World Chat".to_owned())]),
            },
            rx,
        )
    }

    fn get_available_channels(&self) -> impl Iterator<Item = (ChannelId, &str)> {
        self.available_channels.iter().map(|t| (t.0, t.1.as_str()))
    }

    fn is_server_channel(&self, channel: ChannelId) -> bool {
        self.available_channels
            .iter()
            .find(|t| t.0 == channel)
            .is_some()
    }

    pub async fn join_channel(&self, player: AgentKey, channel: ChannelId) {
        todo!();
    }

    pub async fn leave_channel(&self, player: AgentKey, channel: ChannelId) {
        todo!();
    }

    pub async fn message_channel(&self, player: AgentKey, channel: ChannelId, message: String) {
        todo!();
    }

    pub async fn message_player(&self, from: AgentKey, to: AgentKey, message: String) {
        todo!();
    }
}

pub struct ChatActor {
    rx: mpsc::Receiver<ChatCommand>,
    router: MessageRouterActorHandle,
    channels: HashMap<ChannelId, Channel>,
}

impl ChatActor {
    pub fn start(router: MessageRouterActorHandle) -> ChatActorHandle {
        let (tx, rx) = mpsc::channel(CONFIG.max_buffered_messages);

        let mut channels = HashMap::new();
        for ch in GAME_CONFIG.chat.server_channels.iter() {
            channels.insert(
                ch.id,
                Channel {
                    id: ch.id,
                    name: ch.name.clone(),
                    members: Vec::new(),
                },
            );
        }

        let available_channels: Box<[(ChannelId, String)]> =
            channels.values().map(|c| (c.id, c.name.clone())).collect();

        tokio::spawn(async move {
            let actor = Self {
                rx,
                router,
                channels,
            };
            actor.run().await;
        });

        ChatActorHandle {
            tx,
            available_channels,
        }
    }

    async fn run(mut self) {
        info!("Chat actor started");

        loop {
            let command = self.rx.recv().await;
            match command {
                Some(command) => self.handle_command(command).await,
                None => break,
            }
        }
    }

    async fn handle_command(&mut self, command: ChatCommand) {
        match command {
            ChatCommand::JoinChannel { player, channel } => {
                self.add_player_to_channel(player, channel)
            }
            ChatCommand::LeaveChannel { player, channel } => {
                self.remove_player_from_channel(player, channel)
            }
            ChatCommand::SendChannelMessage {
                player,
                channel,
                message,
            } => self.send_channel_message(player, channel, message).await,
            ChatCommand::SendPrivateMessage {
                sender,
                receiver,
                message,
            } => self.send_private_message(sender, receiver, message).await,
        }
    }

    fn add_player_to_channel(&mut self, player: AgentKey, channel_id: ChannelId) {
        if let Some(channel) = self.channels.get_mut(&channel_id) {
            channel.members.push(player);
        }
    }

    fn remove_player_from_channel(&mut self, player: AgentKey, channel_id: ChannelId) {
        if let Some(channel) = self.channels.get_mut(&channel_id) {
            channel.members.retain(|a| *a != player);
        }
    }

    async fn send_channel_message(&self, player: AgentKey, channel_id: ChannelId, message: String) {
    }

    async fn send_private_message(&self, sender: AgentKey, receiver: AgentKey, message: String) {}
}
