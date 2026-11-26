use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    process::{Command, Stdio},
};

fn seal(args: &[&str]) -> Result<Value> {
    let out = Command::new("seal").args(args).output().context("seal")?;
    let response = serde_json::from_slice::<Value>(&out.stdout);
    anyhow::ensure!(
        out.status.success(),
        "seal: {}",
        response
            .as_ref()
            .ok()
            .and_then(|v| v["error"].as_str())
            .unwrap_or(String::from_utf8_lossy(&out.stderr).trim())
    );
    Ok(response?)
}

pub fn run(label: Option<String>, args: Vec<OsString>, machine: bool) -> Result<(Value, i32)> {
    let timestamp = chonk_lib::utc_now()?;
    // Seal derives its ID from the first 48 alphanumeric characters of the message.
    let message = format!(
        "try-{timestamp}-{} {}",
        std::process::id(),
        label.as_deref().unwrap_or("")
    );
    let mark = seal(&["--json", "mark", "--", &message])?;
    let id = mark["id"]
        .as_str()
        .context("seal mark returned no snapshot ID")?;
    let restore = format!("seal restore {id} --dry-run");
    if !machine {
        eprintln!("Snapshot {id}");
    }
    let mut child = Command::new(&args[0]);
    child.args(&args[1..]);
    if machine {
        child.stdout(Stdio::from(std::io::stderr()));
    }
    let status = child.status();
    let diff = seal(&["--json", "diff", id]);
    let code = match &status {
        Ok(status) => status.code().unwrap_or({
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                128 + status.signal().unwrap_or(0)
            }
            #[cfg(not(unix))]
            {
                1
            }
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 127,
        Err(_) => 126,
    };
    let (diff, diff_error) = match diff {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    if !machine {
        if let Some(v) = &diff {
            chonk_lib::report(Ok((v.clone(), 0)), false);
        }
        if let Some(e) = &diff_error {
            eprintln!("seal diff: {e}");
        }
        if let Err(e) = &status {
            eprintln!("{}: {e}", args[0].to_string_lossy());
        }
        eprintln!("Preview restore: {restore}");
    }
    Ok((
        json!({
            "id":id, "timestamp":timestamp, "exit_code":code, "diff":diff, "diff_error":diff_error,
            "command_error":status.err().map(|e| e.to_string()), "restore":restore
        }),
        code,
    ))
}
