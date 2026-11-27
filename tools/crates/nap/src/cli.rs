use chonk_lib::duration;
use clap::{Parser, Subcommand, ValueEnum};
use std::ffi::OsString;

#[derive(Parser)]
#[command(name = "nap", version)]
pub(super) struct Cli {
    #[arg(long, global = true)]
    pub(super) json: bool,
    #[command(subcommand)]
    pub(super) cmd: Cmd,
}
#[derive(Subcommand)]
pub(super) enum Cmd {
    Status,
    /// Manage battery-triggered shutdown
    #[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
    Dive {
        #[command(subcommand)]
        mode: Option<Dive>,
        #[arg(long, required = true)]
        event: bool,
        #[arg(long, requires = "event")]
        dry_run: bool,
    },
    /// Manage the hardware battery charge limit
    Guard {
        #[command(subcommand)]
        mode: Guard,
    },
    /// Switch the built-in panel backlight off or on
    Dark {
        mode: Dark,
    },
    Stay {
        #[arg(required_unless_present="command", conflicts_with="command",value_parser=duration)]
        duration: Option<u64>,
        #[arg(last=true,num_args=1..)]
        command: Vec<OsString>,
    },
    Lid {
        mode: Mode,
    },
}

#[derive(Subcommand)]
pub(super) enum Dive {
    Enable {
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u8).range(10..=50))]
        threshold: u8,
        #[arg(long, value_delimiter = ',', value_parser = service)]
        services: Vec<String>,
    },
    Disable,
    Status,
}

pub(super) fn service(value: &str) -> Result<String, String> {
    if !value.is_empty()
        && !value.starts_with('-')
        && !value.contains("..")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@_.-".contains(&b))
    {
        Ok(value.into())
    } else {
        Err("invalid service name".into())
    }
}

#[derive(Subcommand)]
pub(super) enum Guard {
    Enable {
        #[arg(long, value_enum, default_value = "60")]
        limit: Limit,
    },
    Disable,
    Status,
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum Limit {
    #[value(name = "60")]
    Sixty = 60,
    #[value(name = "80")]
    Eighty = 80,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
pub(super) enum Dark {
    On,
    Off,
    Status,
}

#[derive(Clone, ValueEnum)]
pub(super) enum Mode {
    Sleep,
    Ignore,
    Status,
}
