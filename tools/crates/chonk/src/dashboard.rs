use chonk_lib::{
    bytes,
    metrics::{self, read},
    output,
};
use serde_json::{json, Value};
use std::{path::Path, process::Command};

fn query(cmd: &str, args: &[&str], drift: bool) -> Option<Value> {
    let out = Command::new("timeout")
        .args(["--kill-after=1s", "2s", cmd])
        .args(args)
        .output()
        .ok()?;
    if out.status.success() || (drift && out.status.code() == Some(2)) {
        serde_json::from_slice(&out.stdout).ok()
    } else {
        None
    }
}

fn mood_state(
    hot: bool,
    alert: impl FnOnce() -> bool,
    seal: impl FnOnce() -> bool,
) -> &'static str {
    if hot {
        "hot"
    } else if alert() {
        "alert"
    } else if seal() {
        "seal"
    } else {
        "calm"
    }
}

pub fn mood() -> Value {
    let pss: u64 = metrics::processes().map(|(_, n)| n).sum();
    let total = read("/proc/meminfo").and_then(|s| {
        s.lines().find_map(|l| {
            l.strip_prefix("MemTotal:")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
    });
    let temp = metrics::temperatures().reduce(f64::max);
    let hot = temp.is_some_and(|t| t >= 80.)
        || total.is_some_and(|kb| kb > 0 && pss as f64 >= kb as f64 * 1024. * 0.85);
    let alert = || {
        (Path::new("/var/lib/chonkos/whisker-baseline.json").is_file()
            && query("whisker", &["drift", "--json"], true).is_some_and(|v| {
                v["changes"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|c| c["severity"] == "CRITICAL"))
            }))
            || query("whisker", &["lease", "list", "--json"], false)
                .is_some_and(|v| v["leases"].as_array().is_some_and(|a| !a.is_empty()))
    };
    let seal = || {
        Path::new(chonk_lib::SEAL_STORE).is_dir()
            && query("seal", &["diff", "--json"], false).is_some_and(|v| {
                ["files", "packages"].iter().any(|k| {
                    ["added", "removed", "modified"]
                        .iter()
                        .any(|f| v[k][f].as_array().is_some_and(|a| !a.is_empty()))
                })
            })
    };
    let state = mood_state(hot, alert, seal);
    let zram = metrics::zram_stats().map_or_else(|| "unavailable".into(), |(_, _, n)| bytes(n));
    let temp = temp.map_or_else(|| "unavailable".into(), |t| format!("{t:.1} C"));
    let ports = metrics::listening_ports().map_or_else(
        || "unavailable".into(),
        |(e, l)| format!("{e} exposed, {l} loopback"),
    );
    json!({"text":"CHONK","alt":state,"class":["chonky",format!("state-{state}")],"tooltip":format!("State: {state}\nPSS: {}\nZRAM: {zram}\nTemp: {temp}\nPorts: {ports}",bytes(pss))})
}

pub fn tty(v: &Value) {
    let field = |p: &str| read(p).unwrap_or_else(|| "unknown".into());
    let ips = output("ip", &["-j", "-4", "addr", "show", "scope", "global"])
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
        .unwrap_or_default();
    let address = |tailscale: bool| {
        ips.iter()
            .filter(|i| (i["ifname"] == "tailscale0") == tailscale)
            .find_map(|i| i["addr_info"][0]["local"].as_str())
            .unwrap_or("unavailable")
    };
    let memory = |n: &Value| n.as_u64().map_or_else(|| "unavailable".into(), bytes);
    let battery = v["batteries"]
        .as_array()
        .and_then(|a| a.first())
        .map_or_else(
            || "No battery".into(),
            |b| {
                format!(
                    "{}  {}%",
                    b["status"].as_str().unwrap_or("unknown"),
                    b["percent"]
                )
            },
        );
    let ssh = if chonk_lib::unit_active("ssh.service") {
        "SSH active"
    } else {
        "SSH inactive"
    };
    let lines = vec![
        "ChonkOS Homelab Status".into(),
        "".into(),
        "     /\\_/\\".into(),
        "    ( o.o )   Chonky".into(),
        "     > ^ <".into(),
        "".into(),
        format!("Host       {}", field("/proc/sys/kernel/hostname")),
        format!("LAN        {}", address(false)),
        format!("Tailscale  {}", address(true)),
        "".into(),
        format!("RAM PSS    {}", memory(&v["pss_bytes"])),
        format!("ZRAM       {}", memory(&v["zram"]["allocated_bytes"])),
        format!("Power      {battery}"),
        format!("Max temp   {} C", v["max_temperature_c"]),
        format!(
            "Ports      {} exposed, {} loopback",
            v["ports"]["wildcard"], v["ports"]["loopback"]
        ),
        "".into(),
        format!("Mode       {}", field("/var/lib/chonkos/mode")),
        format!("Profile    {}", field("/var/lib/chonkos/profile")),
    ];
    let mut screen = String::from("\x1b[2J\x1b[H");
    for row in 0..24 {
        let footer = format!("Press Ctrl+Alt+F2 for login | {ssh}");
        let line = if row == 23 {
            footer.as_str()
        } else {
            lines.get(row).map_or("", String::as_str)
        };
        let clean: String = line
            .chars()
            .filter(char::is_ascii)
            .filter(|c| !c.is_control())
            .take(78)
            .collect();
        screen.push_str(&format!(" {clean:<78} "));
        if row < 23 {
            screen.push_str("\r\n");
        }
    }
    print!("{screen}");
}

#[cfg(test)]
mod tests {
    #[test]
    fn mood_precedence() {
        for (hot, alert, seal, expected) in [
            (false, false, false, "calm"),
            (false, false, true, "seal"),
            (false, true, false, "alert"),
            (false, true, true, "alert"),
            (true, false, false, "hot"),
            (true, false, true, "hot"),
            (true, true, false, "hot"),
            (true, true, true, "hot"),
        ] {
            assert_eq!(super::mood_state(hot, || alert, || seal), expected);
        }
    }
}
