use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::{
    fs,
    io::{self, IsTerminal},
    path::Path,
    process::Command,
};

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
    let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let width = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    for (k, v) in rows {
        if color {
            println!("\x1b[38;5;67m{k:width$}\x1b[0m  \x1b[36m{v}\x1b[0m")
        } else {
            println!("{k:width$}  {v}")
        }
    }
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
