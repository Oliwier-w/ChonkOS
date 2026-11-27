use crate::cli::{Lease, Protocol};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::path::Path;

const TABLE: &str = "chonkos_filter";

impl Protocol {
    fn name(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }

    fn set(self) -> &'static str {
        match self {
            Self::Tcp => "chonkos_leases_tcp",
            Self::Udp => "chonkos_leases_udp",
        }
    }
}

fn elements(value: &Value, proto: Protocol) -> Result<Vec<Value>> {
    let set = value["nftables"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|object| &object["set"])
        .find(|set| set["name"] == proto.set() && set["table"] == TABLE && set["family"] == "inet")
        .context("Expected the requested set in nft output")?;
    let Some(items) = set.get("elem") else {
        return Ok(Vec::new());
    };
    let mut leases = Vec::new();
    for item in items.as_array().context("Invalid nft element list")? {
        let element = item.get("elem").unwrap_or(item);
        let port = element.get("val").unwrap_or(element);
        let port = port
            .as_u64()
            .or_else(|| port.as_str()?.parse().ok())
            .filter(|port| (1..=65535).contains(port))
            .context("Invalid port in nft output")?;
        leases.push((
            port,
            json!({
                "proto": proto.name(), "port": port,
                "timeout": element.get("timeout").or_else(|| set.get("timeout")),
                "expires": element.get("expires")
            }),
        ));
    }
    leases.sort_unstable_by_key(|(port, _)| *port);
    Ok(leases.into_iter().map(|(_, lease)| lease).collect())
}

#[cfg(target_os = "linux")]
pub(super) fn run(root_dir: &Path, command: Lease, machine: bool) -> Result<Value> {
    run_with(root_dir, command, machine, |args| {
        chonk_lib::output("nft", args)
    })
}

fn run_with(
    root_dir: &Path,
    command: Lease,
    machine: bool,
    mut nft: impl FnMut(&[&str]) -> Result<String>,
) -> Result<Value> {
    match command {
        Lease::Add {
            port,
            seconds,
            proto,
        } => {
            ensure!(chonk_lib::root_at(root_dir), "Run with sudo");
            nft(&["list", "set", "inet", TABLE, proto.set()]).with_context(|| {
                format!(
                    "Unable to read {}. Enable /usr/share/chonkos/nftables/default.nft through your firewall configuration if the set is missing",
                    proto.set()
                )
            })?;
            nft(&[
                "add",
                "element",
                "inet",
                TABLE,
                proto.set(),
                "{",
                &port.to_string(),
                "timeout",
                &format!("{seconds}s"),
                "}",
            ])?;
            Ok(json!({"status":"added","proto":proto.name(),"port":port,"timeout":seconds}))
        }
        Lease::Revoke { port, proto } => {
            ensure!(chonk_lib::root_at(root_dir), "Run with sudo");
            nft(&[
                "delete",
                "element",
                "inet",
                TABLE,
                proto.set(),
                "{",
                &port.to_string(),
                "}",
            ])?;
            Ok(json!({"status":"revoked","proto":proto.name(),"port":port}))
        }
        Lease::List => {
            let mut leases = Vec::new();
            for proto in [Protocol::Tcp, Protocol::Udp] {
                let output = nft(&["-j", "list", "set", "inet", TABLE, proto.set()])?;
                leases.extend(elements(&serde_json::from_str(&output)?, proto)?);
            }
            if machine {
                return Ok(json!({"leases":leases}));
            }
            let mut rows = vec![("Protocol".into(), "Port  Timeout (s)  Expires (s)".into())];
            for lease in leases {
                let seconds = |key: &str| lease[key].as_u64().map_or("-".into(), |n| n.to_string());
                rows.push((
                    lease["proto"].as_str().unwrap_or_default().into(),
                    format!(
                        "{}  {}  {}",
                        lease["port"],
                        seconds("timeout"),
                        seconds("expires")
                    ),
                ));
            }
            chonk_lib::table(&rows);
            Ok(Value::Null)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use clap::Parser;
    use std::fs;

    #[test]
    fn validates_inputs_and_passes_explicit_nft_arguments() -> Result<()> {
        for (input, expected) in [("10s", 10), ("15m", 900), ("2h", 7200), ("1h30m", 5400)] {
            let cli = Cli::try_parse_from(["whisker", "lease", "add", "65535", "--for", input])?;
            assert!(
                matches!(cli.cmd, crate::Cmd::Lease { command: Lease::Add { port:65535, seconds, .. } } if seconds == expected)
            );
        }
        for (port, duration, proto) in [
            ("0", "10s", "tcp"),
            ("22", "0s", "tcp"),
            ("22", "5s", "tcp"),
            ("22", "48h", "tcp"),
            ("22", "1h; rm -rf /", "tcp"),
            ("65536", "10s", "tcp"),
            ("22", "9s", "tcp"),
            ("22", "24h1s", "tcp"),
            ("22", "10s", "sctp"),
        ] {
            assert!(Cli::try_parse_from([
                "whisker", "lease", "add", port, "--for", duration, "--proto", proto
            ])
            .is_err());
        }
        let cli = Cli::try_parse_from([
            "whisker", "lease", "add", "443", "--for", "1h30m", "--proto", "udp", "--json",
        ])?;
        let crate::Cmd::Lease { command } = cli.cmd else {
            panic!("Expected lease command")
        };
        let root = tempfile::tempdir()?;
        fs::create_dir_all(root.path().join("proc/self"))?;
        fs::write(root.path().join("proc/self/status"), "Uid: 0 0 0 0\n")?;
        let mut calls = Vec::new();
        let result = run_with(root.path(), command, cli.json, |args| {
            calls.push(args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
            Ok(String::new())
        })?;
        assert_eq!(result["timeout"], 5400);
        assert_eq!(
            calls[0],
            ["list", "set", "inet", TABLE, Protocol::Udp.set()]
        );
        assert_eq!(
            calls[1],
            [
                "add",
                "element",
                "inet",
                TABLE,
                Protocol::Udp.set(),
                "{",
                "443",
                "timeout",
                "5400s",
                "}"
            ]
        );
        Ok(())
    }

    #[test]
    fn parses_timed_elements() -> Result<()> {
        let value = json!({"nftables":[{"set":{"family":"inet","table":TABLE,"name":Protocol::Tcp.set(),
            "elem":[{"elem":{"val":8080,"timeout":30,"expires":17}}]}}]});
        assert_eq!(
            elements(&value, Protocol::Tcp)?,
            [json!({"proto":"tcp","port":8080,"timeout":30,"expires":17})]
        );
        Ok(())
    }
}
