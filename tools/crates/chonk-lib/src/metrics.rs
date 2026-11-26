use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::PathBuf,
};

pub fn read(path: impl AsRef<std::path::Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

pub fn entries(path: &str) -> impl DoubleEndedIterator<Item = PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort_unstable();
    paths.into_iter()
}

pub fn pss(s: &str) -> Option<u64> {
    s.lines()
        .find_map(|l| l.strip_prefix("Pss:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

pub fn processes() -> impl Iterator<Item = (PathBuf, u64)> {
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            entry.file_name().to_str()?.parse::<u32>().ok()?;
            let path = entry.path();
            let bytes = pss(&read(path.join("smaps_rollup"))?)?;
            Some((path, bytes))
        })
}

pub fn zram(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.split_whitespace().map(str::parse);
    Some((it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?))
}

pub fn zram_stats() -> Option<(u64, u64, u64)> {
    zram(&read("/sys/block/zram0/mm_stat")?)
}

pub fn zram_active() -> Option<bool> {
    Some(
        read("/proc/swaps")?
            .lines()
            .any(|line| line.split_whitespace().next() == Some("/dev/zram0")),
    )
}

pub fn battery_paths() -> impl Iterator<Item = PathBuf> {
    entries("/sys/class/power_supply").filter(|p| {
        read(p.join("type")).as_deref() == Some("Battery")
            && read(p.join("present")).as_deref() != Some("0")
    })
}

pub fn backlight_paths() -> impl Iterator<Item = PathBuf> {
    entries("/sys/class/backlight")
        .map(|p| p.join("bl_power"))
        .filter(|p| p.is_file())
}

pub fn battery(get: impl Fn(&str) -> Option<String>) -> Value {
    let number = |k: &str| get(k)?.trim().parse::<u64>().ok().map(|v| v as f64);
    let unit = if number("energy_full").is_some() {
        "energy"
    } else {
        "charge"
    };
    let ratio = |a: &str, b: &str| {
        Some(
            100. * number(&format!("{unit}_{a}"))?
                / number(&format!("{unit}_{b}")).filter(|x| *x > 0.)?,
        )
    };
    let watts = number("power_now")
        .map(|p| p / 1e6)
        .or_else(|| Some(number("current_now")? * number("voltage_now")? / 1e12));
    let percent = number("capacity").or_else(|| ratio("now", "full"));
    json!({"status":get("status"),"percent":percent,"health_percent":ratio("full","full_design"),"watts":watts})
}

pub fn temperatures() -> impl Iterator<Item = f64> {
    entries("/sys/class/thermal")
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("thermal_zone"))
        })
        .filter_map(|p| {
            read(p.join("temp"))?
                .parse::<i64>()
                .ok()
                .map(|t| t as f64 / 1000.)
        })
}

fn word(s: &str) -> Option<[u8; 4]> {
    Some(u32::from_str_radix(s, 16).ok()?.to_ne_bytes())
}

pub fn proc_ip(s: &str) -> Option<IpAddr> {
    if !s.is_ascii() {
        return None;
    }
    match s.len() {
        8 => Some(Ipv4Addr::from(word(s)?).into()),
        32 => {
            let mut bytes = [0; 16];
            for (i, chunk) in bytes.chunks_mut(4).enumerate() {
                chunk.copy_from_slice(&word(&s[i * 8..i * 8 + 8])?);
            }
            Some(Ipv6Addr::from(bytes).into())
        }
        _ => None,
    }
}

pub fn listening_ports() -> Option<(usize, usize)> {
    let mut wildcard = BTreeSet::new();
    let mut loopback = BTreeSet::new();
    let mut available = false;
    for (file, tcp) in [
        ("tcp", true),
        ("tcp6", true),
        ("udp", false),
        ("udp6", false),
    ] {
        let Some(data) = read(format!("/proc/net/{file}")) else {
            continue;
        };
        available = true;
        for fields in data.lines().skip(1).map(|line| line.split_whitespace()) {
            let mut fields = fields.skip(1);
            let Some((ip, port)) = fields.next().and_then(|s| s.split_once(':')) else {
                continue;
            };
            let Some(ip) = proc_ip(ip) else { continue };
            let Some(remote) = fields.next() else {
                continue;
            };
            let bound = if tcp {
                fields.next() == Some("0A")
            } else {
                remote.ends_with(":0000")
            };
            if bound && port != "0000" {
                if ip.is_unspecified() {
                    wildcard.insert((tcp, port.to_owned()));
                } else if ip.is_loopback() {
                    loopback.insert((tcp, port.to_owned()));
                }
            }
        }
    }
    available.then_some((wildcard.len(), loopback.len()))
}
