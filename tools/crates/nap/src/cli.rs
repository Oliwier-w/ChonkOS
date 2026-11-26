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

pub(super) fn duration(s: &str) -> Result<u64, String> {
    let mut rest = s;
    let mut total = 0u64;
    while !rest.is_empty() {
        let n = rest.bytes().take_while(u8::is_ascii_digit).count();
        let value = rest[..n].parse::<u64>().map_err(|_| "invalid duration")?;
        let factor = match rest.as_bytes().get(n) {
            Some(b's') => 1,
            Some(b'm') => 60,
            Some(b'h') => 3600,
            _ => return Err("expected s, m or h".into()),
        };
        total = value
            .checked_mul(factor)
            .and_then(|x| total.checked_add(x))
            .ok_or("duration overflow")?;
        rest = &rest[n + 1..];
    }
    if total == 0 {
        Err("duration must be positive".into())
    } else {
        Ok(total)
    }
}
