use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "whisker", version)]
pub(super) struct Cli {
    #[arg(long, global = true)]
    pub(super) json: bool,
    #[command(subcommand)]
    pub(super) cmd: Cmd,
}
#[derive(Subcommand)]
pub(super) enum Cmd {
    Ports {
        #[arg(long)]
        all: bool,
    },
    Kill {
        #[arg(value_parser = clap::value_parser!(u16).range(1..))]
        port: u16,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        yes: bool,
    },
    Net {
        /// Optionally test DNS resolution for this hostname
        host: Option<String>,
    },
    Watch {
        host: String,
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        #[arg(long, default_value_t = 500)]
        interval: u64,
    },
}
