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
    pub creatures_file_path: String,
    pub game_config_path: String,
    pub spawns_file_path: String,
    pub player_despawn_delay: Duration,
    pub database_url: String,
    pub save_interval: Duration,
    /// The site's internal origin. Login is a call to this; there is no SQL fallback, so
    /// if the site is unreachable nobody can log in until it returns.
    pub site_internal_url: String,
    /// The client identity and CA for that call. All three must load or the process
    /// refuses to start — a game server that cannot authenticate to the site can do
    /// nothing useful, and failing at boot beats failing at each player's first login.
    pub internal_tls_cert: String,
    pub internal_tls_key: String,
    pub internal_tls_ca: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            bind_address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 5555)),
            tick_duration: Duration::from_millis(50),
            max_queue_size: 8192,
            max_buffered_messages: 512,
            map_file_path: "assets/map.otbm".to_string(),
            items_file_path: "assets/items.yaml".to_string(),
            creatures_file_path: "assets/creatures.yaml".to_string(),
            game_config_path: "assets/game_conf.yaml".to_string(),
            spawns_file_path: "assets/spawns.yaml".to_string(),
            player_despawn_delay: Duration::from_secs(2), // TODO: move to game config
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://localhost/rustibia".to_string()),
            save_interval: Duration::from_secs(60),
            site_internal_url: std::env::var("SITE_INTERNAL_URL")
                .unwrap_or_else(|_| "https://localhost:8443".to_string()),
            internal_tls_cert: std::env::var("INTERNAL_TLS_CERT")
                .unwrap_or_else(|_| "certs/server.crt".to_string()),
            internal_tls_key: std::env::var("INTERNAL_TLS_KEY")
                .unwrap_or_else(|_| "certs/server.key".to_string()),
            internal_tls_ca: std::env::var("INTERNAL_TLS_CA")
                .unwrap_or_else(|_| "certs/ca.crt".to_string()),
        }
    }
}
