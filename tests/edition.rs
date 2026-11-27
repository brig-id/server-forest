//! forest says what it is and refuses what does not fit it, without any
//! database: these run everywhere.

use std::process::Command;

const KEY: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

fn forest(env: &[(&str, &str)], args: &[&str]) -> (Option<i32>, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_forest"))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("BRIGID_MASTER_KEY", KEY)
        .env("BRIGID_SERVER__DOMAIN", "localhost")
        .envs(env.iter().copied())
        .args(args)
        .output()
        .expect("run forest");
    (
        out.status.code(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn the_executable_says_its_version() {
    let (code, out, _) = forest(&[], &["--version"]);
    assert_eq!(code, Some(0));
    assert_eq!(out.trim(), format!("forest {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_node_needs_the_address_of_a_database() {
    let (code, _, err) = forest(&[], &[]);
    assert_eq!(code, Some(1));
    assert!(err.contains("database.url_file"), "{err}");
}

/// The connection to the database carries every row: without the
/// operator's written choice, it verifies the database's certificate.
#[test]
fn a_database_reached_without_a_verified_certificate_is_refused() {
    let url = "postgres://forest:secret@db.internal/forest?sslmode=require";
    let (code, _, err) = forest(&[("BRIGID_DATABASE__URL", url)], &[]);
    assert_eq!(code, Some(1));
    assert!(err.contains("sslmode=require"), "{err}");
    assert!(
        !err.contains("secret"),
        "the password is never repeated: {err}"
    );
    let (code, _, err) = forest(&[("BRIGID_DATABASE__URL", url)], &["init"]);
    assert_eq!(code, Some(2));
    assert!(!err.contains("secret"), "{err}");
}

/// A backup is the file of a SQLite database: forest has none.
#[test]
fn backup_and_restore_are_the_databases_own() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("backup.db");
    let url = [(
        "BRIGID_DATABASE__URL",
        "postgres://forest@db.internal/forest",
    )];
    let (code, _, err) = forest(&url, &["backup", file.to_str().unwrap()]);
    assert_eq!(code, Some(2));
    assert!(err.contains("pg_dump"), "{err}");
    assert!(!file.exists(), "nothing was written");
}

/// The certificate of forest is held by what stands in front of its nodes.
#[test]
fn forest_obtains_no_certificate_itself() {
    let (code, _, err) = forest(
        &[
            ("BRIGID_SERVER__ACME", "true"),
            (
                "BRIGID_DATABASE__URL",
                "postgres://forest@db.internal/forest",
            ),
        ],
        &[],
    );
    assert_eq!(code, Some(1));
    assert!(err.contains("server.acme"), "{err}");
}
