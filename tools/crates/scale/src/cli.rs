use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "scale", version)]
pub(super) struct Cli {
    #[arg(long, global = true)]
    pub(super) json: bool,
    #[command(subcommand)]
    pub(super) command: Cmd,
}
#[derive(Subcommand)]
pub(super) enum Cmd {
    Ram {
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    Boot {
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    Pkg {
        #[arg(long, default_value_t = 15)]
        top: usize,
    },
    Dir {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value_t = 15)]
        top: usize,
    },
}
