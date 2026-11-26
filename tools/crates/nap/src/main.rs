use anyhow::Result;
use clap::Parser;
use serde_json::Value;

mod cli;
#[cfg(any(target_os = "linux", test))]
mod policy;
#[cfg(any(test, target_os = "linux"))]
use chonk_lib::metrics::battery;
#[cfg(test)]
use cli::duration;
use cli::Cli;
#[cfg(target_os = "linux")]
use cli::Cmd;
#[cfg(target_os = "linux")]
use cli::Mode;
#[cfg(any(test, target_os = "linux"))]
fn thermal(kind: &str, temp: &str) -> Option<Value> {
    Some(
        serde_json::json!({"type":kind.trim(),"celsius":temp.trim().parse::<i64>().ok()? as f64/1000.}),
    )
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use anyhow::{ensure, Context};
    use chonk_lib::{LidMode, LID_CONFIG};
    use serde_json::json;
    use std::{fs, os::unix::process::ExitStatusExt, process::Command};
    fn lid() -> Result<Value> {
        match fs::read_to_string(LID_CONFIG) {
            Ok(v) => Ok(json!(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!("system default")),
            Err(e) => Err(e.into()),
        }
    }
    fn entries(path: &str) -> Result<Vec<std::path::PathBuf>> {
        match fs::read_dir(path) {
            Ok(dir) => dir.map(|e| Ok(e?.path())).collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.into()),
        }
    }
    pub fn run(cmd: Cmd, machine: bool) -> Result<(Value, i32)> {
        let v = match cmd {
            Cmd::Guard { mode } => {
                if !matches!(mode, super::cli::Guard::Status) {
                    ensure!(chonk_lib::root(), "Run with sudo");
                }
                super::policy::guard(std::path::Path::new("/"), mode)?
            }
            Cmd::Dark { mode } => {
                if mode != super::cli::Dark::Status {
                    ensure!(chonk_lib::root(), "Run with sudo");
                }
                super::policy::dark(std::path::Path::new("/"), mode)?
            }
            Cmd::Status => {
                let mut batteries = vec![];
                let mut ac = vec![];
                let mut temps = vec![];
                for p in entries("/sys/class/power_supply")? {
                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                    let read = |k: &str| {
                        fs::read_to_string(p.join(k))
                            .ok()
                            .map(|s| s.trim().to_owned())
                    };
                    if name.starts_with("BAT") {
                        let mut b = battery(read);
                        b["name"] = json!(name);
                        batteries.push(b)
                    } else if name.starts_with("AC") || name.starts_with("ADP") {
                        ac.push(json!({"name":name,"online":read("online").map(|x|x=="1")}))
                    }
                }
                for p in entries("/sys/class/thermal")? {
                    if p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .starts_with("thermal_zone")
                    {
                        if let (Ok(k), Ok(t)) = (
                            fs::read_to_string(p.join("type")),
                            fs::read_to_string(p.join("temp")),
                        ) {
                            temps.extend(thermal(&k, &t))
                        }
                    }
                }
                json!({"battery_status":if batteries.is_empty(){"no battery"}else{"present"},"batteries":batteries,"ac":ac,"temperatures":temps,"lid":lid()?})
            }
            Cmd::Lid { mode: Mode::Status } => json!({"lid":lid()?}),
            Cmd::Lid { mode } => {
                let (label, mode) = match mode {
                    Mode::Sleep => ("sleep", LidMode::Sleep),
                    _ => ("ignore", LidMode::Ignore),
                };
                ensure!(chonk_lib::root(), "Run with: sudo nap lid {label}");
                chonk_lib::apply_lid(&mode)?;
                json!({"lid":mode.action()})
            }
            Cmd::Stay { duration, command } => {
                let mut c = Command::new("systemd-inhibit");
                c.args([
                    "--what=idle:sleep:handle-lid-switch",
                    "--who=nap",
                    "--why=ChonkOS nap stay",
                    "--mode=block",
                    "--",
                ]);
                if let Some(s) = duration {
                    c.arg("sleep").arg(s.to_string());
                } else {
                    c.args(command);
                }
                if machine {
                    use std::os::fd::AsFd;
                    c.stdout(std::io::stderr().as_fd().try_clone_to_owned()?);
                }
                let status = c.status().context("systemd-inhibit")?;
                let code = status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(0));
                return Ok((json!({"exit_code":code}), code));
            }
        };
        Ok((v, 0))
    }
}
fn main() {
    let cli = Cli::parse();
    #[cfg(target_os = "linux")]
    let result = linux::run(cli.cmd, cli.json);
    #[cfg(not(target_os = "linux"))]
    let result: Result<(Value, i32)> = Err(anyhow::anyhow!("nap requires Linux"));
    std::process::exit(chonk_lib::report(result, cli.json))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durations() {
        assert_eq!(duration("1h30m"), Ok(5400));
        for s in ["", "0s", "-1s", "2h1", "1ms", "18446744073709551615h"] {
            assert!(duration(s).is_err())
        }
    }
    #[test]
    fn power() {
        for unit in ["energy", "charge"] {
            let v = battery(|k| match k {
                "current_now" => Some("2000000".into()),
                "voltage_now" => Some("12000000".into()),
                "power_now" if unit == "energy" => Some("24000000".into()),
                _ if k == format!("{unit}_now") => Some("40".into()),
                _ if k == format!("{unit}_full") => Some("80".into()),
                _ if k == format!("{unit}_full_design") => Some("100".into()),
                _ => None,
            });
            assert_eq!(v["percent"], 50.);
            assert_eq!(v["health_percent"], 80.);
            assert_eq!(v["watts"], 24.);
        }
    }
    #[test]
    fn zone() {
        assert_eq!(thermal("cpu", "42375\n").unwrap()["celsius"], 42.375);
        assert!(thermal("cpu", "?").is_none())
    }
}
