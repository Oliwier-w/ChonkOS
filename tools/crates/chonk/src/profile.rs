use crate::cli::Selection;
use anyhow::{ensure, Context, Result};
use chonk_lib::{metrics, output, LID_CONFIG};
use serde_json::{json, Value};
use std::{fs, io, path::Path};

const PROFILE_PATH: &str = "/var/lib/chonkos/profile";
const GOVERNOR_POLICIES: &str = "/sys/devices/system/cpu/cpufreq";

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| path.display().to_string()),
    }
}

fn read_profile(path: &Path) -> Result<String> {
    let Some(contents) = read_optional(path)? else {
        return Ok("default".into());
    };
    let name = contents.trim();
    ensure!(
        matches!(name, "dev" | "battery" | "homelab" | "default"),
        "Invalid profile in {}: {name:?}",
        path.display()
    );
    Ok(name.into())
}

#[cfg(any(test, target_os = "linux"))]
fn write_profile(path: &Path, name: &str) -> Result<()> {
    chonk_lib::write_public_config(path, &format!("{name}\n"))
}

fn setting(contents: &str, key: &str) -> Option<String> {
    contents.lines().rev().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        let value = value.trim();
        (name.trim() == key && !value.is_empty()).then(|| value.to_owned())
    })
}

fn status() -> Result<Value> {
    let (lid, fallback) = match fs::read_to_string(LID_CONFIG) {
        Ok(contents) => (contents, "system default"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (String::new(), "system default"),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => (String::new(), "unreadable"),
        Err(e) => return Err(e).context(LID_CONFIG),
    };
    let tlp = output("tlp-stat", &["-s"])
        .ok()
        .and_then(|s| setting(&s, "TLP profile").or_else(|| setting(&s, "Mode")));
    let profile = read_profile(Path::new(PROFILE_PATH))?;
    let battery = metrics::battery_paths().next().is_some();
    let guard = output("nap", &["guard", "status", "--json"])
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["enabled"].as_bool());
    let governors: Vec<_> = metrics::entries(GOVERNOR_POLICIES)
        .filter_map(|p| metrics::read(p.join("scaling_governor")))
        .collect();
    let governor = governors.first();
    let zram = metrics::zram_active();
    let lid_action = setting(&lid, "HandleLidSwitch");
    let lid_ac = setting(&lid, "HandleLidSwitchExternalPower");
    let lid_consistent = (profile != "default").then(|| {
        let expected = if profile == "homelab" {
            "ignore"
        } else {
            "suspend"
        };
        lid_action.as_deref() == Some(expected) && lid_ac.as_deref() == Some(expected)
    });
    let governor_consistent = (!governors.is_empty() && profile != "default").then(|| {
        governors.iter().all(|g| match profile.as_str() {
            "battery" => matches!(
                g.as_str(),
                "powersave" | "schedutil" | "ondemand" | "conservative"
            ),
            _ => matches!(
                g.as_str(),
                "performance" | "powersave" | "schedutil" | "ondemand"
            ),
        })
    });
    let guard_consistent = if profile == "homelab" { guard } else { None };
    let consistent = if profile == "default" {
        None
    } else {
        Some(
            [lid_consistent, guard_consistent, governor_consistent, zram]
                .into_iter()
                .flatten()
                .all(|ok| ok),
        )
    };
    Ok(json!({
        "profile": profile,
        "lid": lid_action.as_deref().unwrap_or(fallback),
        "lid_external_power": lid_ac.as_deref().unwrap_or(fallback),
        "tlp_mode": tlp,
        "scaling_governor": governor,
        "scaling_governors": governors,
        "battery_present": battery,
        "guard_enabled": guard,
        "zram_active": zram,
        "consistent": consistent,
        "checks": {"lid":lid_consistent,"guard":guard_consistent,"governor":governor_consistent,"zram":zram},
    }))
}

#[cfg(target_os = "linux")]
fn homelab_addresses(result: &mut Value) {
    use std::net::IpAddr;

    let tailscale: Vec<IpAddr> = output("tailscale", &["ip"])
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|address| address.parse().ok())
        .collect();
    let lan: Vec<IpAddr> = output("ip", &["-o", "address", "show", "up", "scope", "global"])
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let interface = fields.nth(1)?;
            let family = fields.next()?;
            if interface == "tailscale0" || !matches!(family, "inet" | "inet6") {
                return None;
            }
            let (address, _) = fields.next()?.split_once('/')?;
            address.parse().ok()
        })
        .filter(|address| !tailscale.contains(address))
        .collect();
    result["lan_ips"] = json!(lan);
    result["tailscale_ips"] = json!(tailscale);
}

fn dark_action(selection: Selection, dark: bool) -> Result<Option<&'static str>> {
    ensure!(
        !dark || matches!(selection, Selection::Homelab),
        "--dark requires the homelab profile"
    );
    Ok(match selection {
        Selection::Homelab if dark => Some("on"),
        Selection::Dev => Some("off"),
        _ => None,
    })
}

pub fn execute(selection: Selection, dark: bool) -> Result<Value> {
    let dark_mode = dark_action(selection, dark)?;
    #[cfg(not(target_os = "linux"))]
    let _ = dark_mode;
    let name = match selection {
        Selection::Status => return status(),
        Selection::Dev => "dev",
        Selection::Battery => "battery",
        Selection::Homelab => "homelab",
    };
    ensure!(chonk_lib::root(), "Run with: sudo chonk profile {name}");
    #[cfg(target_os = "linux")]
    {
        use chonk_lib::LidMode;
        use std::os::fd::AsRawFd;

        let profile_path = Path::new(PROFILE_PATH);
        let state_dir = profile_path.parent().unwrap();
        fs::create_dir_all(state_dir)?;
        let lock = fs::File::open(state_dir)?;
        // Serialize profile switches while applying the settings and publishing the marker.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io::Error::last_os_error()).context("Locking the ChonkOS state directory");
        }
        let homelab = matches!(selection, Selection::Homelab);
        let battery_profile = matches!(selection, Selection::Battery);
        let lid = if homelab {
            LidMode::Ignore
        } else {
            LidMode::Sleep
        };
        let tlp = if battery_profile {
            "bat"
        } else if homelab {
            "ac"
        } else {
            "start"
        };
        let battery = metrics::battery_paths().next().is_some();
        let find = |name: &str| {
            std::env::var_os("PATH")
                .into_iter()
                .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
                .map(|dir| dir.join(name))
                .find(|p| p.is_file())
        };
        let tlp_bin = find("tlp");
        let nap_bin = find("nap");
        for path in [LID_CONFIG, PROFILE_PATH] {
            read_optional(Path::new(path))?;
        }
        let mut warnings = Vec::new();
        if !battery {
            warnings.push("No battery detected; skipping nap guard".to_owned());
        } else if battery_profile {
            warnings.push("Set a battery charge limit with sudo nap guard enable".to_owned());
        }
        chonk_lib::apply_lid(&lid)?;
        let mut apply =
            |bin: Option<&Path>, name: &str, args: &[&str]| match bin.and_then(Path::to_str) {
                Some(cmd) => {
                    if let Err(e) = output(cmd, args) {
                        warnings.push(format!("{name} {}: {e:#}", args.join(" ")));
                    }
                }
                None => warnings.push(format!("{name} is unavailable")),
            };
        apply(tlp_bin.as_deref(), "tlp", &[tlp]);
        if battery && homelab {
            apply(nap_bin.as_deref(), "nap", &["guard", "enable"]);
        }
        if let Some(mode) = dark_mode {
            apply(nap_bin.as_deref(), "nap", &["dark", mode]);
        }
        write_profile(profile_path, name)?;
        let mut result = json!({"profile":name,"lid":lid.action(),"warnings":warnings});
        if homelab {
            homelab_addresses(&mut result);
        }
        Ok(result)
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("Requires Linux");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homelab_blanking_requires_explicit_cli_flag() -> Result<()> {
        use crate::cli::{Cli, Cmd};
        use clap::Parser;
        for (args, expected) in [
            (vec!["chonk", "profile", "homelab"], None),
            (vec!["chonk", "profile", "homelab", "--dark"], Some("on")),
            (vec!["chonk", "profile", "dev"], Some("off")),
        ] {
            let Cmd::Profile { profile, dark } = Cli::try_parse_from(args)?.command else {
                unreachable!()
            };
            assert_eq!(dark_action(profile, dark)?, expected);
        }
        for selection in [Selection::Dev, Selection::Battery, Selection::Status] {
            assert!(dark_action(selection, true).is_err());
        }
        Ok(())
    }

    #[test]
    fn profile_file() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("chonkos/profile");
        assert_eq!(read_profile(&path)?, "default");
        for name in ["dev", "battery", "homelab"] {
            write_profile(&path, name)?;
            assert_eq!(fs::read(&path)?, format!("{name}\n").as_bytes());
            assert_eq!(read_profile(&path)?, name);
        }
        fs::write(&path, " battery \r\n")?;
        assert_eq!(read_profile(&path)?, "battery");
        fs::write(&path, "unknown\n")?;
        let error = read_profile(&path).unwrap_err().to_string();
        assert!(error.contains("Invalid profile") && error.contains("unknown"));
        assert_eq!(fs::read_to_string(path)?, "unknown\n");
        Ok(())
    }
}
