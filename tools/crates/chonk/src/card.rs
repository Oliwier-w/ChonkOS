use chonk_lib::{
    bytes,
    metrics::{self, read},
    output, SEAL_STORE,
};
use serde_json::{json, Value};
use std::io::{self, IsTerminal};

pub fn collect() -> anyhow::Result<Value> {
    let os = read("/usr/lib/os-release");
    let version = os
        .as_deref()
        .and_then(|s| super::release_field(s, "VERSION_ID"));
    let uptime =
        read("/proc/uptime").and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok());
    let boot = output("systemd-analyze", &["time"])
        .ok()
        .and_then(|s| s.lines().next().map(str::to_owned));
    let load = boot
        .is_none()
        .then(|| {
            read("/proc/loadavg")
                .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        })
        .flatten();
    let (pss, samples) = metrics::processes().fold((0, 0), |(total, count), (_, bytes)| {
        (total + bytes, count + 1)
    });
    let zram = metrics::zram_stats().map(|(original, compressed, allocated)| {
        json!({"original_bytes":original,"compressed_bytes":compressed,"allocated_bytes":allocated,
            "ratio":(compressed > 0).then(|| original as f64 / compressed as f64)})
    });
    let batteries: Vec<_> = metrics::battery_paths()
        .map(|path| {
            let mut battery = metrics::battery(|key| read(path.join(key)));
            battery["name"] = json!(path.file_name());
            battery
        })
        .collect();
    let ports = metrics::listening_ports()
        .map(|(wildcard, loopback)| json!({"wildcard":wildcard,"loopback":loopback}));
    let seal = metrics::entries(SEAL_STORE).rev().find_map(|p| {
        if !p
            .file_name()?
            .to_str()?
            .as_bytes()
            .first()?
            .is_ascii_digit()
            || !p.is_dir()
        {
            return None;
        }
        let manifest: Value = serde_json::from_str(&read(p.join("manifest.json"))?).ok()?;
        (manifest["id"].as_str()? == p.file_name()?.to_str()?)
            .then(|| json!({"id":manifest["id"],"message":manifest["message"]}))
    });
    Ok(json!({
        "schema_version":1, "timestamp":chonk_lib::utc_now()?, "version":version,
        "kernel":read("/proc/sys/kernel/osrelease"), "uptime_seconds":uptime,
        "pss_bytes":(samples > 0).then_some(pss), "pss_processes":samples, "zram":zram,
        "boot":boot, "load_average":load, "batteries":batteries,
        "max_temperature_c":metrics::temperatures().reduce(f64::max),
        "ports":ports, "last_seal":seal
    }))
}

pub fn render(v: &Value, plain: bool) {
    let show = |value: &Value| value.as_str().unwrap_or("—").to_owned();
    let pss = v["pss_bytes"]
        .as_u64()
        .map(bytes)
        .unwrap_or_else(|| "—".into());
    let zram = v["zram"]["allocated_bytes"]
        .as_u64()
        .map(|allocated| {
            let ratio = v["zram"]["ratio"]
                .as_f64()
                .map(|r| format!(" · {r:.2}×"))
                .unwrap_or_default();
            format!("{}{}", bytes(allocated), ratio)
        })
        .unwrap_or_else(|| "—".into());
    let mut power = Vec::new();
    if let Some(batteries) = v["batteries"].as_array() {
        for b in batteries {
            let mut parts = vec![show(&b["name"]), show(&b["status"])];
            if let Some(p) = b["percent"].as_f64() {
                parts.push(format!("{p:.0}%"));
            }
            if let Some(w) = b["watts"].as_f64() {
                parts.push(format!("{w:.1} W"));
            }
            power.push(parts.join(" "));
        }
    }
    if let Some(t) = v["max_temperature_c"].as_f64() {
        power.push(format!("{t:.1} °C max"));
    }
    let uptime = v["uptime_seconds"]
        .as_f64()
        .map(|s| {
            let minutes = s as u64 / 60;
            format!(
                "{}d {:02}h {:02}m",
                minutes / 1440,
                minutes / 60 % 24,
                minutes % 60
            )
        })
        .unwrap_or_else(|| "—".into());
    let boot = v["boot"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("load {}", show(&v["load_average"])));
    let ports = if v["ports"].is_null() {
        "—".into()
    } else {
        format!(
            "{} wildcard · {} loopback",
            v["ports"]["wildcard"], v["ports"]["loopback"]
        )
    };
    let mut rows = [
        (
            "ChonkOS",
            format!(
                "{} · Linux {} · up {uptime}",
                show(&v["version"]),
                show(&v["kernel"])
            ),
        ),
        ("Memory", format!("PSS {pss} · ZRAM {zram}")),
        ("Boot", boot),
        (
            "Power",
            if power.is_empty() {
                "—".into()
            } else {
                power.join(" · ")
            },
        ),
        ("Ports", ports),
        ("Seal", show(&v["last_seal"]["id"])),
    ]
    .map(|(key, value)| (key.to_owned(), value));
    if io::stdout().is_terminal() && !plain {
        let seal = [
            "    ▄▄▄▄    ",
            "  ▄█•  •█▄  ",
            " ▐█  ᴥ  ██▄ ",
            "  ▀████████▌",
            " ▄▄███████▀ ",
            "   ▀▀▀▀▀    ",
        ];
        for ((key, _), line) in rows.iter_mut().zip(seal) {
            *key = format!("{line}  {key}");
        }
    }
    chonk_lib::table_with_color(&rows, !plain);
}
