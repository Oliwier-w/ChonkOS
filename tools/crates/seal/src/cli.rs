use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "seal", version)]
pub(super) struct Cli {
    #[arg(long, global = true)]
    pub(super) json: bool,
    #[arg(long, global = true, default_value = "/var/lib/chonkos/seals")]
    pub(super) store: PathBuf,
    #[command(subcommand)]
    pub(super) cmd: Cmd,
}
#[derive(Subcommand)]
pub(super) enum Cmd {
    Mark {
        message: String,
    },
    List,
    Diff {
        id: Option<String>,
    },
    Restore {
        id: String,
        #[arg(required_unless_present = "dry_run")]
        target_path: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Export {
        file: Option<PathBuf>,
    },
}
