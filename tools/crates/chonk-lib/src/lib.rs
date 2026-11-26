use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
    process::Command,
};

pub const LID_CONFIG: &str = "/etc/systemd/logind.conf.d/90-chonkos-lid.conf";
pub const SEAL_STORE: &str = "/var/lib/chonkos/seals";
pub mod metrics;

pub fn utc_now() -> Result<String> {
    Ok(time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?)
}

pub fn unit_active(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", unit])
        .output()
        .is_ok_and(|o| o.status.success())
}

pub enum LidMode {
    Sleep,
    Ignore,
}

impl LidMode {
    pub fn action(&self) -> &'static str {
        match self {
            Self::Sleep => "suspend",
            Self::Ignore => "ignore",
        }
    }

    #[cfg(any(test, target_os = "linux"))]
    fn config(&self) -> String {
        let action = self.action();
        format!("[Login]\nHandleLidSwitch={action}\nHandleLidSwitchExternalPower={action}\n")
    }
}

pub fn write_public_config(path: &Path, contents: &str) -> Result<()> {
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(contents.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .with_context(|| path.display().to_string())?;
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn apply_lid(mode: &LidMode) -> Result<()> {
    write_public_config(Path::new(LID_CONFIG), &mode.config())?;
    if !Command::new("/usr/bin/systemctl")
        .args(["reload", "systemd-logind"])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("Configuration saved. It takes effect when systemd-logind next reads it.");
    }
    Ok(())
}

pub fn output(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .with_context(|| cmd.to_owned())?;
    ensure!(
        out.status.success(),
        "{}: {}",
        cmd,
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?)
}
pub fn root() -> bool {
    fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|v| v.split_whitespace().nth(1))
        == Some("0")
}
pub fn bytes(n: u64) -> String {
    if n >= 1 << 30 {
        format!("{:.2} GiB", n as f64 / 1073741824.)
    } else {
        format!("{:.2} MiB", n as f64 / 1048576.)
    }
}
pub fn json(v: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string(v)?);
    Ok(())
}
pub fn table(rows: &[(String, String)]) {
    table_with_color(rows, true);
}

pub fn table_with_color(rows: &[(String, String)], styled: bool) {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    write_table(
        &mut output,
        rows,
        styled && stdout.is_terminal(),
        std::env::var_os("NO_COLOR").as_deref(),
    )
    .expect("writing stdout");
}

fn write_table(
    mut output: impl Write,
    rows: &[(String, String)],
    styled_terminal: bool,
    no_color: Option<&std::ffi::OsStr>,
) -> io::Result<()> {
    let color = styled_terminal && no_color.is_none();
    let width = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    for (k, v) in rows {
        if color {
            writeln!(output, "\x1b[38;5;67m{k:width$}\x1b[0m  \x1b[36m{v}\x1b[0m")?;
        } else {
            writeln!(output, "{k:width$}  {v}")?;
        }
    }
    Ok(())
}
pub fn size(path: &Path) -> Result<u64> {
    if matches!(path.to_str(), Some("/proc" | "/sys" | "/dev")) {
        return Ok(0);
    }
    let m = fs::symlink_metadata(path)?;
    if m.is_symlink() {
        return Ok(0);
    }
    if !m.is_dir() {
        return Ok(m.len());
    }
    let mut total = 0;
    for e in fs::read_dir(path)? {
        total += size(&e?.path())?
    }
    Ok(total)
}

pub fn report(result: Result<(serde_json::Value, i32)>, machine: bool) -> i32 {
    let (value, code) = match result {
        Ok(v) => v,
        Err(e) if machine => (serde_json::json!({"error":format!("{e:#}")}), 1),
        Err(e) => {
            eprintln!("{e:#}");
            return 1;
        }
    };
    if machine {
        println!("{value}")
    } else {
        fn rows(key: &str, v: &serde_json::Value, out: &mut Vec<(String, String)>) {
            match v {
                serde_json::Value::Object(m) if !m.is_empty() => {
                    for (k, item) in m {
                        let label = if key.is_empty() {
                            k.clone()
                        } else {
                            format!("{key} {k}")
                        };
                        rows(&label, item, out);
                    }
                }
                serde_json::Value::Array(a) if !a.is_empty() => {
                    for item in a {
                        rows(key, item, out)
                    }
                }
                _ => out.push((
                    key.into(),
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string()),
                )),
            }
        }
        let mut data = Vec::new();
        rows("", &value, &mut data);
        table(&data);
    }
    code
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_color_suppresses_ansi_on_a_terminal() {
        let rows = [("Ports".into(), "2".into())];
        let mut output = Vec::new();
        super::write_table(&mut output, &rows, true, None).unwrap();
        assert!(output.windows(2).any(|w| w == b"\x1b["));
        output.clear();
        super::write_table(&mut output, &rows, true, Some("1".as_ref())).unwrap();
        assert_eq!(output, b"Ports  2\n");
    }
    use super::LidMode;

    #[test]
    fn lid_configuration() {
        assert_eq!(
            LidMode::Sleep.config(),
            "[Login]\nHandleLidSwitch=suspend\nHandleLidSwitchExternalPower=suspend\n"
        );
        assert_eq!(
            LidMode::Ignore.config(),
            "[Login]\nHandleLidSwitch=ignore\nHandleLidSwitchExternalPower=ignore\n"
        );
    }
}
