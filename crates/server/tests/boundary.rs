//! The one thing this merge actively weakens.
//!
//! While the projects were separate repositories it was impossible for the server to
//! call the site's internals — the crate simply was not there. In one workspace
//! nothing stops `use rustibia_site::db::characters;`, which would bypass the HTTP
//! boundary the login-over-REST work depends on. Only this test does.

#[test]
fn the_server_does_not_depend_on_the_site_crate() {
    // Read the manifest at runtime, relative to the working directory. Cargo sets a
    // test binary's cwd to its package root, so "Cargo.toml" is this crate's manifest.
    //
    // Deliberately NOT `concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")`: that is
    // resolved at compile time and baked in as an absolute path, so a cached binary
    // keeps pointing at the old location after the directory is moved or renamed —
    // which made this test fail spuriously during the monorepo merge.
    let manifest = std::fs::read_to_string("Cargo.toml").expect("reading the server manifest");

    assert!(
        !manifest.contains("rustibia-site"),
        "crates/server must not depend on crates/site. The boundary between them is an \
         HTTP API; linking the crates would let a refactor quietly erase it."
    );
}
