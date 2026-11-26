use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::Value;
use std::ffi::OsString;

#[derive(Parser)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}
#[derive(Subcommand)]
enum Cmd {
    Status,
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
#[derive(Clone, ValueEnum)]
enum Mode {
    Sleep,
    Ignore,
    Status,
}

fn duration(s: &str) -> Result<u64, String> {
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
#[cfg(any(test, target_os = "linux"))]
fn battery(get: impl Fn(&str) -> Option<String>) -> Value {
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
    serde_json::json!({"status":get("status"),"percent":ratio("now","full"),"health_percent":ratio("full","full_design"),"watts":watts})
}
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
    use serde_json::json;
    use std::{fs, io::Write, os::unix::process::ExitStatusExt, path::Path, process::Command};
    const LID: &str = "/etc/systemd/logind.conf.d/90-chonkos-lid.conf";
    fn lid() -> Result<Value> {
        match fs::read_to_string(LID) {
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
                let (label, action) = match mode {
                    Mode::Sleep => ("sleep", "suspend"),
                    _ => ("ignore", "ignore"),
                };
                ensure!(chonk_lib::root(), "Run with: sudo nap lid {label}");
                let dir = Path::new(LID).parent().unwrap();
                fs::create_dir_all(dir)?;
                let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
                writeln!(
                    tmp,
                    "[Login]\nHandleLidSwitch={action}\nHandleLidSwitchExternalPower={action}"
                )?;
                tmp.as_file().sync_all()?;
                tmp.persist(LID)?;
                fs::File::open(dir)?.sync_all()?;
                if !Command::new("/usr/bin/systemctl")
                    .args(["reload", "systemd-logind"])
                    .output()
                    .is_ok_and(|o| o.status.success())
                {
                    eprintln!(
                        "Configuration saved. It takes effect when systemd-logind next reads it."
                    );
                }
                json!({"lid":action})
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
