use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "chonk", version)]
pub(super) struct Cli {
    #[arg(long, global = true)]
    pub(super) json: bool,
    #[command(subcommand)]
    pub(super) command: Cmd,
}
#[derive(Subcommand)]
pub(super) enum Cmd {
    Doctor,
    /// Show hardware health
    Card {
        #[arg(long)]
        plain: bool,
    },
    /// Snapshot system configuration, run a command, and show changes
    Try {
        #[arg(long)]
        label: Option<String>,
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<std::ffi::OsString>,
    },
    /// Show or change the active ChonkOS profile
    Profile {
        profile: Selection,
        /// Run nap dark on when selecting homelab
        #[arg(long)]
        dark: bool,
    },
    Update {
        #[arg(long)]
        yes: bool,
    },
    Clean {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        dev: bool,
    },
    Docs {
        tool: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum Selection {
    Dev,
    Battery,
    Homelab,
    Status,
}
