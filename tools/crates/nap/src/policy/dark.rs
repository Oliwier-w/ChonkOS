use super::{entries, remove_if_present, system_command, write};
use crate::cli::Dark;
use anyhow::{Context, Result};
use chonk_lib::{
    metrics::{battery, read},
    write_public_config,
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, io, path::Path};

const STATE: &str = "var/lib/chonkos/backlight-prev";
const WIFI_DIR: &str = "etc/NetworkManager/conf.d";
const WIFI: &str = "etc/NetworkManager/conf.d/90-chonkos-wifi.conf";
const CONSOLE: &str = "sys/module/kernel/parameters/consoleblank";

fn watts(root: &Path) -> Option<f64> {
    entries(&root.join("sys/class/power_supply"))
        .filter(|p| {
            read(p.join("type")).as_deref() == Some("Battery")
                && read(p.join("present")).as_deref() != Some("0")
        })
        .filter_map(|p| battery(|key| read(p.join(key)))["watts"].as_f64())
        .reduce(|a, b| a + b)
}

fn saved(path: &Path) -> Result<BTreeMap<String, u32>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| path.display().to_string()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e).with_context(|| path.display().to_string()),
    }
}

fn backlights(root: &Path) -> impl Iterator<Item = (String, std::path::PathBuf)> {
    entries(&root.join("sys/class/backlight")).filter_map(|path| {
        let name = path.file_name()?.to_str()?.to_owned();
        (path.join("bl_power").is_file() || path.join("brightness").is_file())
            .then_some((name, path))
    })
}

fn backlight(
    root: &Path,
    mode: Dark,
    actions: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let state = root.join(STATE);
    let mut previous = saved(&state)?;
    let mut restored = false;
    for (name, path) in backlights(root) {
        let power = path.join("bl_power");
        let brightness = path.join("brightness");
        if mode == Dark::On {
            if power.is_file() {
                // 4 is FB_BLANK_POWERDOWN; 1 (VESA_SUSPEND) is ignored by
                // several drivers and would leave the panel lit.
                match write(&power, "4") {
                    Ok(()) => {
                        actions.push(format!("{name}: bl_power=4"));
                        continue;
                    }
                    Err(e) if brightness.is_file() => {
                        warnings.push(format!("{e:#}; using brightness"))
                    }
                    Err(e) => return Err(e),
                }
            }
            if !previous.contains_key(&name) {
                let level = read(&brightness)
                    .and_then(|s| s.parse::<u32>().ok())
                    .with_context(|| format!("Reading brightness from {}", brightness.display()))?;
                previous.insert(name.clone(), level);
                write_public_config(&state, &serde_json::to_string(&previous)?)?;
            }
            write(&brightness, "0")?;
            actions.push(format!("{name}: brightness=0"));
        } else {
            if power.is_file() {
                match write(&power, "0") {
                    Ok(()) => actions.push(format!("{name}: bl_power=0")),
                    Err(e) if previous.contains_key(&name) && brightness.is_file() => {
                        warnings.push(format!("{e:#}; restoring brightness"));
                    }
                    Err(e) => return Err(e),
                }
            }
            if let Some(level) = previous.get(&name).filter(|_| brightness.is_file()) {
                write(&brightness, &level.to_string())?;
                actions.push(format!("{name}: brightness restored to {level}"));
                previous.remove(&name);
                restored = true;
            }
        }
    }
    if mode == Dark::Off {
        if previous.is_empty() {
            remove_if_present(&state)?;
        } else {
            if restored {
                write_public_config(&state, &serde_json::to_string(&previous)?)?;
            }
            warnings.push("Saved brightness retained for unavailable backlights".into());
        }
    }
    Ok(())
}

fn blank_console(root: &Path, actions: &mut Vec<String>, warnings: &mut Vec<String>) {
    if write(&root.join(CONSOLE), "60").is_ok() {
        actions.push("consoleblank=60".into());
        return;
    }
    let configure_tty = || -> io::Result<bool> {
        let tty = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty0")?;
        std::process::Command::new("setterm")
            .args(["--blank", "1"])
            .env("TERM", "linux")
            .stdin(tty.try_clone()?)
            .stdout(tty)
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
    };
    if root == Path::new("/") && configure_tty().unwrap_or(false) {
        actions.push("setterm --blank 1".into());
    } else {
        warnings.push(
            "Set the kernel parameter consoleblank=60 to blank the TTY after one minute".into(),
        );
    }
}

fn reload_network_manager(root: &Path, actions: &mut Vec<String>) {
    if root == Path::new("/")
        && matches!(
            system_command(root, "nmcli", &["general", "reload"]),
            Ok(true)
        )
    {
        actions.push("NetworkManager reloaded".into());
    }
}

fn wifi(root: &Path, actions: &mut Vec<String>, warnings: &mut Vec<String>) -> Result<()> {
    if root.join(WIFI_DIR).is_dir() {
        write_public_config(&root.join(WIFI), "[connection]\nwifi.powersave = 2\n")?;
        actions.push("NetworkManager Wi-Fi power saving disabled in the drop-in".into());
        reload_network_manager(root, actions);
    }
    for iface in entries(&root.join("sys/class/net"))
        .filter(|p| p.join("wireless").is_dir() || p.join("phy80211").exists())
    {
        let Some(name) = iface.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        match system_command(root, "iw", &["dev", name, "set", "power_save", "off"]) {
            Ok(true) => actions.push(format!("{name}: Wi-Fi power saving disabled")),
            Ok(false) => break,
            Err(e) => warnings.push(format!("{name}: iw set power_save off: {e:#}")),
        }
    }
    Ok(())
}

pub(in crate::policy) fn run(root: &Path, mode: Dark) -> Result<Value> {
    let mut actions = Vec::new();
    let mut warnings = Vec::new();
    let before = (mode == Dark::On).then(|| watts(root)).flatten();
    match mode {
        Dark::On => {
            backlight(root, mode, &mut actions, &mut warnings)?;
            if backlights(root).next().is_none() {
                warnings.push("No controllable backlight detected".into());
            }
            blank_console(root, &mut actions, &mut warnings);
            wifi(root, &mut actions, &mut warnings)?;
        }
        Dark::Off => {
            backlight(root, mode, &mut actions, &mut warnings)?;
            if remove_if_present(&root.join(WIFI))? {
                actions.push("NetworkManager Wi-Fi drop-in removed".into());
                reload_network_manager(root, &mut actions);
            }
            for iface in entries(&root.join("sys/class/net"))
                .filter(|p| p.join("wireless").is_dir() || p.join("phy80211").exists())
            {
                let Some(name) = iface.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                match system_command(root, "iw", &["dev", name, "set", "power_save", "on"]) {
                    Ok(true) => actions.push(format!("{name}: Wi-Fi power saving restored")),
                    Ok(false) => break,
                    Err(e) => warnings.push(format!("{name}: iw set power_save on: {e:#}")),
                }
            }
        }
        Dark::Status => {}
    }
    let devices: Vec<_> = backlights(root)
        .map(|(name, path)| {
            let power = read(path.join("bl_power")).and_then(|s| s.parse::<u32>().ok());
            let brightness = read(path.join("brightness")).and_then(|s| s.parse::<u32>().ok());
            let dark = match (power, brightness) {
                (Some(power), _) if power > 0 => Some(true),
                (_, Some(brightness)) => Some(brightness == 0),
                (power, None) => power.map(|power| power != 0),
            };
            json!({"name":name,"bl_power":power,"brightness":brightness,"dark":dark})
        })
        .collect();
    let enabled = (!devices.is_empty() && devices.iter().all(|v| v["dark"].is_boolean()))
        .then(|| devices.iter().all(|v| v["dark"] == true));
    let mut result = json!({
        "enabled":enabled, "backlights":devices, "wifi_drop_in":root.join(WIFI).is_file(),
        "consoleblank":read(root.join(CONSOLE)).and_then(|s|s.parse::<u32>().ok())
    });
    if mode != Dark::Status {
        result["actions"] = json!(actions);
        result["warnings"] = json!(warnings);
    }
    if mode == Dark::On {
        let after = watts(root);
        result["power_watts"] = json!({"before":before,"after":after,
            "delta":before.zip(after).map(|(before, after)| after - before)});
    }
    Ok(result)
}
