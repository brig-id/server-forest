//! brig·id `forest`: the server for a company, on several nodes.
//!
//! The server and its commands are `brigid-server` (core), the same for every
//! edition: a node of forest answers what seedling and grove answer. What
//! makes it forest is that several of them serve one PostgreSQL database and
//! stand in for one another: a node creates nothing when it starts (`forest
//! init` does, once, before the nodes), keeps nothing in its memory that
//! another node needs, and is replaced without a sign-in in progress being
//! lost. The design is `forest.md` in brig-id/spec.

use std::path::PathBuf;

use clap::Parser;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_ui.rs"));
}

/// forest as an edition of the server.
const EDITION: brigid_server::Edition = brigid_server::Edition {
    name: "forest",
    version: env!("CARGO_PKG_VERSION"),
    ui: brigid_server::EmbeddedUi {
        files: embedded::FILES,
        csp_hashes: embedded::CSP_HASHES,
    },
    // A node is configured by its environment (`BRIGID_*`), and gets its key
    // and the address of its database from files the platform mounts
    // (`BRIGID_MASTER_KEY_FILE`, `BRIGID_DATABASE__URL_FILE`).
    config_path: None,
    key_path: None,
    // The key of several nodes is a mounted secret: never sealed by a machine.
    sealed_key_path: None,
    key_hint: " (a secret mounted in every node, and BRIGID_MASTER_KEY_FILE=<its path>)",
    root_hint: ": the image runs it as nonroot (65532).",
};

#[derive(Parser)]
#[command(
    name = "forest",
    version,
    about = "brig·id server for a company, on several nodes"
)]
struct Cli {
    /// Path to a TOML configuration file. Optional: a node is normally
    /// configured by `BRIGID_*` environment variables alone.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<brigid_server::NodeCommand>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    brigid_server::run_node(&EDITION, cli.config, cli.command).await;
}
