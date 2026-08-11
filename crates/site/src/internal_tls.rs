//! The TLS configuration for the internal listener.
//!
//! This is what makes `/internal/*` safe to serve without an auth extractor: the
//! `WebPkiClientVerifier` below refuses any connection whose peer does not present a
//! certificate signed by our CA, so a request reaching a handler has already proved it
//! came from the game server.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use rustls::{
    RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer},
    server::WebPkiClientVerifier,
};

#[derive(Debug, thiserror::Error)]
pub enum TlsConfigError {
    #[error("cannot read {0}: {1}")]
    Read(String, std::io::Error),
    #[error("{0} contains no certificates")]
    NoCertificates(String),
    #[error("{0} contains no private key")]
    NoPrivateKey(String),
    #[error("{0} is not a usable certificate authority: {1}")]
    InvalidCa(String, rustls::Error),
    #[error("the client certificate verifier could not be built: {0}")]
    Verifier(#[from] rustls::server::VerifierBuilderError),
    #[error("invalid TLS configuration: {0}")]
    Rustls(#[from] rustls::Error),
}

#[derive(Debug, Clone)]
pub struct InternalTlsPaths {
    pub cert: String,
    pub key: String,
    pub client_ca: String,
}

impl InternalTlsPaths {
    pub fn from_env() -> Self {
        Self {
            cert: std::env::var("INTERNAL_TLS_CERT").unwrap_or_else(|_| "certs/site.crt".into()),
            key: std::env::var("INTERNAL_TLS_KEY").unwrap_or_else(|_| "certs/site.key".into()),
            client_ca: std::env::var("INTERNAL_TLS_CLIENT_CA")
                .unwrap_or_else(|_| "certs/ca.crt".into()),
        }
    }
}

pub fn server_config(paths: &InternalTlsPaths) -> Result<Arc<ServerConfig>, TlsConfigError> {
    let certs = load_certs(&paths.cert)?;
    let key = load_key(&paths.key)?;

    let mut roots = RootCertStore::empty();
    for ca in load_certs(&paths.client_ca)? {
        roots
            .add(ca)
            .map_err(|e| TlsConfigError::InvalidCa(paths.client_ca.clone(), e))?;
    }

    let verifier = WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()?;

    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs, key)?;

    // The game server's reqwest client negotiates HTTP/2 when offered, and axum-server
    // reads this list to decide what to hand the connection to.
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(Arc::new(config))
}

fn load_certs(path: &str) -> Result<Vec<CertificateDer<'static>>, TlsConfigError> {
    let mut reader = BufReader::new(open(path)?);
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<_, _>>()
        .map_err(|e| TlsConfigError::Read(path.to_string(), e))?;

    if certs.is_empty() {
        return Err(TlsConfigError::NoCertificates(path.to_string()));
    }
    Ok(certs)
}

fn load_key(path: &str) -> Result<PrivateKeyDer<'static>, TlsConfigError> {
    let mut reader = BufReader::new(open(path)?);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| TlsConfigError::Read(path.to_string(), e))?
        .ok_or_else(|| TlsConfigError::NoPrivateKey(path.to_string()))
}

fn open(path: &str) -> Result<File, TlsConfigError> {
    File::open(Path::new(path)).map_err(|e| TlsConfigError::Read(path.to_string(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::SiteConfig, state::AppState};
    use sqlx::PgPool;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};

    struct Certs {
        dir: std::path::PathBuf,
    }

    impl Certs {
        fn generate(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "rustibia-site-tls-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            rustibia_certgen::generate_bundle(&dir).unwrap();
            Self { dir }
        }

        fn path(&self, name: &str) -> String {
            self.dir.join(name).display().to_string()
        }

        fn paths(&self) -> InternalTlsPaths {
            InternalTlsPaths {
                cert: self.path("site.crt"),
                key: self.path("site.key"),
                client_ca: self.path("ca.crt"),
            }
        }
    }

    impl Drop for Certs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_generated_bundle_builds_a_server_config() {
        let certs = Certs::generate("builds");
        assert!(server_config(&certs.paths()).is_ok());
    }

    #[test]
    fn a_missing_certificate_is_an_error_and_never_a_downgrade() {
        let certs = Certs::generate("missing-cert");
        let mut paths = certs.paths();
        paths.cert = certs.path("does-not-exist.crt");

        let err = server_config(&paths).expect_err("a missing certificate must not be tolerated");
        assert!(matches!(err, TlsConfigError::Read(_, _)), "got {err:?}");
    }

    #[test]
    fn a_missing_client_ca_is_an_error() {
        let certs = Certs::generate("missing-ca");
        let mut paths = certs.paths();
        paths.client_ca = certs.path("does-not-exist.crt");

        assert!(
            server_config(&paths).is_err(),
            "without the CA there is nothing to verify clients against, so serving \
             would mean serving unauthenticated"
        );
    }

    #[test]
    fn a_file_that_is_not_pem_is_an_error() {
        let certs = Certs::generate("garbage");
        let garbage = certs.path("garbage.crt");
        std::fs::write(&garbage, b"this is not a certificate").unwrap();

        let mut paths = certs.paths();
        paths.cert = garbage;

        let err = server_config(&paths).expect_err("garbage must not parse");
        assert!(
            matches!(err, TlsConfigError::NoCertificates(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn the_key_must_match_the_certificate() {
        let site = Certs::generate("mismatch-a");
        let other = Certs::generate("mismatch-b");

        let paths = InternalTlsPaths {
            cert: site.path("site.crt"),
            key: other.path("site.key"),
            client_ca: site.path("ca.crt"),
        };

        assert!(
            server_config(&paths).is_err(),
            "a certificate and key from different bundles must be rejected at startup, \
             not at the first handshake"
        );
    }

    /// Spawns the real internal router behind the real mTLS listener on an ephemeral
    /// port. Returns the address; the server task dies with the test.
    async fn serve_internal(pool: PgPool, certs: &Certs) -> SocketAddr {
        let listener =
            TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))).unwrap();
        // Tokio refuses to adopt a blocking socket, and it refuses by panicking inside
        // the spawned task — which shows up as a connection refusal at the client,
        // indistinguishable from a working TLS rejection. Hence also
        // `assert_the_listener_is_live` below.
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();

        let config = server_config(&certs.paths()).unwrap();
        let app = axum::Router::new()
            .nest("/internal", crate::api::internal::router())
            .with_state(AppState {
                pool,
                config: SiteConfig::load("config.yaml").unwrap(),
            });

        tokio::spawn(async move {
            axum_server::from_tcp_rustls(
                listener,
                axum_server::tls_rustls::RustlsConfig::from_config(config),
            )
            .unwrap()
            .serve(app.into_make_service())
            .await
            .unwrap();
        });

        addr
    }

    fn client_with_identity(certs: &Certs) -> reqwest::Client {
        let mut identity = std::fs::read(certs.dir.join("server.crt")).unwrap();
        identity.extend_from_slice(&std::fs::read(certs.dir.join("server.key")).unwrap());

        reqwest::Client::builder()
            .use_rustls_tls()
            .add_root_certificate(
                reqwest::Certificate::from_pem(&std::fs::read(certs.dir.join("ca.crt")).unwrap())
                    .unwrap(),
            )
            .identity(reqwest::Identity::from_pem(&identity).unwrap())
            .build()
            .unwrap()
    }

    /// One account with one character, returning both ids.
    async fn a_character(pool: &PgPool) -> (i32, i32) {
        use crate::db::{accounts::create_account, characters};

        let account = create_account(pool, "player@example.com", "hunter2hunter2")
            .await
            .unwrap();
        let template = SiteConfig::load("config.yaml").unwrap().new_character;
        let character_id = characters::create(
            pool,
            account.id,
            "Rizael",
            crate::domain::vocation::Vocation::Paladin,
            crate::domain::sex::Sex::Male,
            &template,
        )
        .await
        .unwrap();

        (account.id, character_id)
    }

    async fn a_token(pool: &PgPool, account_id: i32) -> String {
        use time::{Duration, OffsetDateTime};

        let token = format!("token-{}", uuid::Uuid::now_v7());
        sqlx::query(
            "INSERT INTO auth_tokens (token_hash, account_id, valid_until) VALUES ($1, $2, $3)",
        )
        .bind(crate::auth::token::hash_token(&token))
        .bind(account_id)
        .bind(OffsetDateTime::now_utc() + Duration::seconds(60))
        .execute(pool)
        .await
        .unwrap();
        token
    }

    fn redeem_url(addr: SocketAddr) -> String {
        format!("https://localhost:{}/internal/sessions/redeem", addr.port())
    }

    /// Proves the listener is actually accepting before a test concludes that a client
    /// was *rejected*. Without this, a server that failed to start makes every negative
    /// TLS test pass — which is exactly what happened the first time these were written.
    async fn assert_the_listener_is_live(
        addr: SocketAddr,
        certs: &Certs,
        token: &str,
        character_id: i32,
    ) {
        let response = client_with_identity(certs)
            .post(redeem_url(addr))
            .json(&rustibia_contract::RedeemRequest {
                auth_token: token.to_string(),
                character_id,
            })
            .send()
            .await
            .expect("a properly certificated client must get through");

        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    /// The only test that proves the certificates actually work. Everything else in
    /// this crate reaches the handler with TLS bypassed.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_client_with_our_certificate_can_redeem_over_tls(pool: PgPool) {
        let certs = Certs::generate("e2e-ok");
        let (account_id, character_id) = a_character(&pool).await;
        let token = a_token(&pool, account_id).await;
        let addr = serve_internal(pool, &certs).await;

        let response = client_with_identity(&certs)
            .post(redeem_url(addr))
            .json(&rustibia_contract::RedeemRequest {
                auth_token: token,
                character_id,
            })
            .send()
            .await
            .expect("the handshake and request must both succeed");

        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let record: rustibia_contract::CharacterRecord = response.json().await.unwrap();
        assert_eq!(record.id, character_id);
        assert_eq!(record.name, "Rizael");
    }

    /// The other half: without a client certificate the request never reaches a handler.
    /// This is the whole justification for `/internal/*` having no auth extractor.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_client_without_a_certificate_is_rejected_by_tls(pool: PgPool) {
        let certs = Certs::generate("e2e-no-cert");
        let (account_id, character_id) = a_character(&pool).await;
        let liveness_token = a_token(&pool, account_id).await;
        let token = a_token(&pool, account_id).await;
        let addr = serve_internal(pool.clone(), &certs).await;

        assert_the_listener_is_live(addr, &certs, &liveness_token, character_id).await;

        // Trusts our CA, so the *server* verifies fine; it simply has no identity of
        // its own to present. That isolates client authentication as the thing failing.
        let anonymous = reqwest::Client::builder()
            .use_rustls_tls()
            .add_root_certificate(
                reqwest::Certificate::from_pem(&std::fs::read(certs.dir.join("ca.crt")).unwrap())
                    .unwrap(),
            )
            .build()
            .unwrap();

        let result = anonymous
            .post(redeem_url(addr))
            .json(&rustibia_contract::RedeemRequest {
                auth_token: token.clone(),
                character_id,
            })
            .send()
            .await;

        assert!(
            result.is_err(),
            "an uncertificated client must fail at the TLS layer, got {result:?}"
        );

        let hash = crate::auth::token::hash_token(&token);
        let remaining: Option<String> =
            sqlx::query_scalar("SELECT token_hash FROM auth_tokens WHERE token_hash = $1")
                .bind(&hash)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert_eq!(
            remaining.as_ref(),
            Some(&hash),
            "the request must not have reached the handler, so its token is unspent"
        );
    }

    /// A certificate signed by some other CA is no better than none. Without this, the
    /// verifier could be misconfigured to accept any well-formed certificate and the
    /// test above would still pass.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_certificate_from_another_ca_is_rejected(pool: PgPool) {
        let ours = Certs::generate("e2e-foreign-ours");
        let theirs = Certs::generate("e2e-foreign-theirs");
        let (account_id, character_id) = a_character(&pool).await;
        let liveness_token = a_token(&pool, account_id).await;
        let token = a_token(&pool, account_id).await;
        let addr = serve_internal(pool, &ours).await;

        assert_the_listener_is_live(addr, &ours, &liveness_token, character_id).await;

        // Presents `theirs`, but still trusts *our* CA for the server side, so only the
        // client certificate is foreign.
        let mut identity = std::fs::read(theirs.dir.join("server.crt")).unwrap();
        identity.extend_from_slice(&std::fs::read(theirs.dir.join("server.key")).unwrap());
        let impostor = reqwest::Client::builder()
            .use_rustls_tls()
            .add_root_certificate(
                reqwest::Certificate::from_pem(&std::fs::read(ours.dir.join("ca.crt")).unwrap())
                    .unwrap(),
            )
            .identity(reqwest::Identity::from_pem(&identity).unwrap())
            .build()
            .unwrap();

        let result = impostor
            .post(redeem_url(addr))
            .json(&rustibia_contract::RedeemRequest {
                auth_token: token,
                character_id,
            })
            .send()
            .await;

        assert!(
            result.is_err(),
            "a certificate from an unknown CA must be refused, got {result:?}"
        );
    }
}
