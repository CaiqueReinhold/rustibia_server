use once_cell::sync::Lazy;
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};

pub static CONFIG: Lazy<ServerConfig> = Lazy::new(ServerConfig::default);

pub struct ServerConfig {
    pub bind_address: SocketAddr,
    pub tick_duration: Duration,
    pub max_queue_size: usize,
    pub max_buffered_messages: usize,
    pub map_file_path: String,
    pub items_file_path: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            bind_address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 5555)),
            tick_duration: Duration::from_millis(50),
            max_queue_size: 2048,
            max_buffered_messages: 512,
            map_file_path: "assets/map.otbm".to_string(),
            items_file_path: "assets/items.yaml".to_string(),
        }
    }
}
