use super::{dark, remove_if_present};
use crate::cli::{service, Dark, Dive};
use anyhow::{ensure, Context, Result};
use chonk_lib::{
    metrics::{battery, read},
    write_public_config,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, io, path::Path};

const STATE: &str = "var/lib/chonkos/nap-dive.enabled";

#[derive(Deserialize, Serialize)]
struct Config {
    threshold_percent: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    services: Vec<String>,
}

impl Config {
    fn validate(&self) -> Result<()> {
        ensure!(
            (10..=50).contains(&self.threshold_percent),
            "Threshold must be between 10 and 50 percent"
        );
        for name in &self.services {
            service(name).map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }

    fn load(path: &Path) -> Result<Option<Self>> {
        match fs::read(path) {
            Ok(bytes) => {
                let config: Self =
                    serde_json::from_slice(&bytes).context("Reading nap dive settings")?;
                config.validate()?;
                Ok(Some(config))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

fn power(root_dir: &Path) -> Result<(bool, Vec<Value>)> {
    let supply = match fs::read_dir(root_dir.join("sys/class/power_supply")) {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((false, Vec::new())),
        Err(error) => return Err(error.into()),
    };
    let mut batteries = Vec::new();
    let mut ac_online = false;
    for entry in supply {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let get = |key: &str| read(path.join(key));
        if get("type").as_deref() == Some("Battery") || name.to_string_lossy().starts_with("BAT") {
            if get("present").as_deref() != Some("0") {
                batteries.push(battery(get));
            }
        } else {
            ac_online |= get("online").as_deref() == Some("1");
        }
    }
    Ok((ac_online, batteries))
}

#[cfg(target_os = "linux")]
pub(crate) fn run(root_dir: &Path, mode: Option<Dive>, dry_run: bool) -> Result<Value> {
    run_with(root_dir, mode, dry_run, |command, args| {
        chonk_lib::output(command, args).map(|_| ())
    })
}

fn run_with(
    root_dir: &Path,
    mode: Option<Dive>,
    dry_run: bool,
    mut execute: impl FnMut(&str, &[&str]) -> Result<()>,
) -> Result<Value> {
    let path = root_dir.join(STATE);
    let config = match mode {
        Some(Dive::Enable {
            threshold,
            services,
        }) => {
            ensure!(chonk_lib::root_at(root_dir), "Run with sudo");
            let config = Config {
                threshold_percent: threshold,
                services,
            };
            config.validate()?;
            write_public_config(&path, &serde_json::to_string(&config)?)?;
            return Ok(json!({"status":"enabled","config":config}));
        }
        Some(Dive::Disable) => {
            ensure!(chonk_lib::root_at(root_dir), "Run with sudo");
            remove_if_present(&path)?;
            return Ok(json!({"status":"disabled"}));
        }
        mode => {
            let Some(config) = Config::load(&path)? else {
                return Ok(json!({"status":"disabled"}));
            };
            if matches!(mode, Some(Dive::Status)) {
                return Ok(json!({"status":"enabled","config":config}));
            }
            config
        }
    };
    let (ac_online, batteries) = power(root_dir)?;
    if batteries.is_empty() || ac_online {
        let status = if batteries.is_empty() {
            "no_battery"
        } else {
            "ac_online"
        };
        return Ok(json!({"status":status}));
    }
    let percent = batteries
        .iter()
        .filter(|battery| battery["status"] == "Discharging")
        .filter_map(|battery| battery["percent"].as_f64())
        .filter(|percent| (0.0..=100.0).contains(percent))
        .reduce(f64::min);
    let Some(percent) = percent else {
        return Ok(json!({"status":"no_discharging_reading"}));
    };
    if percent > f64::from(config.threshold_percent) {
        let mut value =
            json!({"status":"on_battery_monitoring","battery_percent":percent,"dry_run":dry_run});
        if dry_run {
            value["steps"] = json!(["Blank the screen"]);
        } else {
            value["screen"] = dark::run(root_dir, Dark::On, false)?;
        }
        return Ok(value);
    }

    let mut steps = vec![vec!["seal", "mark", "powerloss"]];
    for service in &config.services {
        steps.push(vec!["systemctl", "stop", service]);
    }
    steps.push(vec!["sync"]);
    steps.push(vec!["systemctl", "poweroff"]);
    if dry_run {
        return Ok(json!({"status":"dry_run","battery_percent":percent,"steps":steps}));
    }

    ensure!(chonk_lib::root_at(root_dir), "Run with sudo");
    let mut warnings = Vec::new();
    for step in &steps[..steps.len() - 1] {
        if let Err(error) = execute(step[0], &step[1..]) {
            warnings.push(format!("{}: {error:#}", step.join(" ")));
        }
    }
    if let Err(error) = execute("systemctl", &["poweroff"]) {
        for warning in &warnings {
            eprintln!("{warning}");
        }
        return Err(error);
    }
    Ok(json!({"status":"poweroff_requested","battery_percent":percent,"warnings":warnings}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_dive_commands() {
        for args in [
            &["dive", "status", "--json"][..],
            &[
                "dive",
                "enable",
                "--services",
                "foo.service,worker@1",
                "--threshold",
                "10",
            ][..],
            &["dive", "--event", "--dry-run", "--json"][..],
            &["--json", "dive", "disable"][..],
        ] {
            let result =
                crate::Cli::try_parse_from(["nap"].into_iter().chain(args.iter().copied()));
            assert!(result.is_ok(), "{:?}", result.err());
        }
        for args in [
            &["dive"][..],
            &["dive", "--dry-run"][..],
            &["dive", "--event", "status"][..],
            &["dive", "enable", "--services=bad..service"][..],
            &["dive", "enable", "--services=-bad.service"][..],
            &["dive", "enable", "--threshold", "51"][..],
        ] {
            assert!(
                crate::Cli::try_parse_from(["nap"].into_iter().chain(args.iter().copied()))
                    .is_err()
            );
        }
    }

    #[test]
    fn dry_run_and_shutdown_at_threshold() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path();
        let forbidden = |_: &str, _: &[&str]| panic!("Unexpected external command");
        assert_eq!(
            run_with(path, None, false, forbidden)?["status"],
            "disabled"
        );
        fs::create_dir_all(path.join("proc/self"))?;
        fs::write(path.join("proc/self/status"), "Uid: 0 0 0 0\n")?;
        run_with(
            path,
            Some(Dive::Enable {
                threshold: 20,
                services: vec!["web.service".into()],
            }),
            false,
            forbidden,
        )?;
        assert_eq!(
            run_with(path, None, false, forbidden)?["status"],
            "no_battery"
        );
        let battery = path.join("sys/class/power_supply/BAT0");
        fs::create_dir_all(&battery)?;
        fs::write(battery.join("status"), "Discharging\n")?;
        fs::write(battery.join("capacity"), "20\n")?;
        let ac = path.join("sys/class/power_supply/AC0");
        fs::create_dir_all(&ac)?;
        fs::write(ac.join("online"), "1\n")?;
        assert_eq!(
            run_with(path, None, false, forbidden)?["status"],
            "ac_online"
        );
        fs::write(ac.join("online"), "0\n")?;
        let dry = run_with(path, None, true, forbidden)?;
        assert_eq!(dry["status"], "dry_run");
        let mut calls = Vec::new();
        let result = run_with(path, None, false, |cmd, args| {
            calls.push(
                std::iter::once(cmd)
                    .chain(args.iter().copied())
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            );
            ensure!(cmd != "seal", "Seal unavailable");
            Ok(())
        })?;
        assert_eq!(dry["steps"], json!(calls));
        assert_eq!(result["status"], "poweroff_requested");
        assert_eq!(result["warnings"].as_array().unwrap().len(), 1);

        fs::write(battery.join("capacity"), "21\n")?;
        let wifi = path.join("etc/NetworkManager/conf.d/90-chonkos-wifi.conf");
        fs::create_dir_all(wifi.parent().unwrap())?;
        fs::write(&wifi, "Existing settings")?;
        let backlight = path.join("sys/class/backlight/panel");
        fs::create_dir_all(&backlight)?;
        fs::write(backlight.join("brightness"), "12")?;
        assert_eq!(
            run_with(path, None, false, forbidden)?["status"],
            "on_battery_monitoring"
        );
        assert_eq!(fs::read_to_string(backlight.join("brightness"))?, "0");
        assert_eq!(fs::read_to_string(wifi)?, "Existing settings");
        Ok(())
    }
}
