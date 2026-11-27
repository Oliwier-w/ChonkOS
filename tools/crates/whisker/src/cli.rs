use clap::{Parser, Subcommand, ValueEnum};

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
    Lease {
        #[command(subcommand)]
        command: Lease,
    },
    Baseline {
        #[arg(long)]
        save: bool,
    },
    Drift,
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

#[derive(Subcommand)]
pub(super) enum Lease {
    Add {
        #[arg(value_parser = clap::value_parser!(u16).range(1..))]
        port: u16,
        #[arg(long = "for", value_parser = lease_duration)]
        seconds: u64,
        #[arg(long, value_enum, default_value = "tcp")]
        proto: Protocol,
    },
    List,
    Revoke {
        #[arg(value_parser = clap::value_parser!(u16).range(1..))]
        port: u16,
        #[arg(long, value_enum, default_value = "tcp")]
        proto: Protocol,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum Protocol {
    Tcp,
    Udp,
}

fn lease_duration(value: &str) -> Result<u64, String> {
    let seconds = chonk_lib::duration(value)?;
    if (10..=86400).contains(&seconds) {
        Ok(seconds)
    } else {
        Err("lease duration must be between 10s and 24h".into())
    }
}
