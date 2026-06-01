use std::path::PathBuf;

use anyhow::Result;
use brutal_search::daemon::{default_socket_path, run_daemon};
use brutal_search::index::PreloadMode;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about = "Resident daemon for brutally fast warm search.")]
struct Cli {
    #[arg(long, default_value = ".brutal-index")]
    index: PathBuf,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long, default_value = "aggressive")]
    preload: PreloadMode,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = cli
        .socket
        .unwrap_or_else(|| default_socket_path(&cli.index));
    eprintln!(
        "brutal-searchd: index={} socket={} preload={:?}",
        cli.index.display(),
        socket.display(),
        cli.preload
    );
    run_daemon(cli.index, socket, cli.preload).await
}
