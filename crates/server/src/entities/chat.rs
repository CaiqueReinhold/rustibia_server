use crate::entities::agent::AgentKey;

pub type ChannelId = u16;

#[derive(Debug)]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    pub members: Vec<AgentKey>,
}

#[derive(Debug, Clone, Copy)]
pub enum ChatMessageType {
    Local,
    Channel,
    Private,
}
