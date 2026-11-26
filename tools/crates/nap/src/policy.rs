mod dark;
mod guard;
#[cfg(test)]
mod tests;

use crate::cli::{Dark, Guard};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

fn entries(path: &Path) -> impl Iterator<Item = PathBuf> {
    let mut entries: Vec<_> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort_unstable();
    entries.into_iter()
}

fn write(path: &Path, value: &str) -> Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .and_then(|mut file| file.write_all(value.as_bytes()))
        .with_context(|| format!("Writing {}", path.display()))
}

fn remove_if_present(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("Removing {}", path.display())),
    }
}

fn system_command(root: &Path, program: &str, args: &[&str]) -> Result<bool> {
    if root != Path::new("/") {
        return Ok(false);
    }
    let output = match std::process::Command::new(program).args(args).output() {
        Ok(output) => output,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(
        output.status.success(),
        "{} {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(true)
}

pub(super) fn guard(root: &Path, mode: Guard) -> Result<Value> {
    guard::run(root, mode)
}

pub(super) fn dark(root: &Path, mode: Dark) -> Result<Value> {
    dark::run(root, mode)
}
