mod card;
mod dashboard;
mod profile;
mod session;
mod trial;

use anyhow::{anyhow, ensure, Result};
use chonk_lib::{output, root};
use clap::Parser;
use serde_json::{json, Value};
#[cfg(any(test, target_os = "linux"))]
use std::path::Component;
use std::{
    fs, io,
    path::Path,
    process::{Command, Stdio},
};
#[cfg(target_os = "linux")]
use std::{io::Write, path::PathBuf, time::Duration};
mod cli;
use cli::{Cli, Cmd};
fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(cmd)
        .args(args)
        .stdout(Stdio::from(io::stderr()))
        .status()?;
    ensure!(status.success(), "{cmd} {args:?}: {status}");
    Ok(())
}
#[cfg(any(test, target_os = "linux"))]
fn safe(home: &Path, path: &Path) -> bool {
    path.strip_prefix(home).is_ok_and(|rel| {
        rel.components().next().is_some()
            && rel.components().all(|x| matches!(x, Component::Normal(_)))
    }) && matches!(
        path.file_name().and_then(|s| s.to_str()),
        Some("target" | "node_modules")
    )
}
#[cfg(target_os = "linux")]
fn opened_dir(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)?)
}
#[cfg(target_os = "linux")]
fn mount_id(dir: &fs::File) -> Result<u64> {
    use std::os::fd::AsRawFd;
    let info = fs::read_to_string(format!("/proc/self/fdinfo/{}", dir.as_raw_fd()))?;
    Ok(info
        .lines()
        .find_map(|line| line.strip_prefix("mnt_id:"))
        .ok_or_else(|| anyhow!("Missing mount ID"))?
        .trim()
        .parse()?)
}
#[cfg(target_os = "linux")]
fn restrict_removal(home: &fs::File) -> Result<()> {
    use landlock::{
        AccessFs, CompatLevel, Compatible, PathBeneath, Ruleset, RulesetAttr, RulesetCreatedAttr,
    };
    let rights = AccessFs::RemoveDir | AccessFs::RemoveFile;
    Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(rights)?
        .create()?
        .add_rule(PathBeneath::new(home, rights))?
        .restrict_self()?;
    Ok(())
}
#[cfg(target_os = "linux")]
fn walk_contents(dir: &fs::File, mount: u64, remove: bool) -> Result<u64> {
    use std::os::fd::AsRawFd;
    let mut total = 0;
    for entry in fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            let child = opened_dir(&entry.path())?;
            ensure!(
                mount_id(&child)? == mount,
                "Refusing to cross mount at {:?}",
                entry.file_name()
            );
            total += walk_contents(&child, mount, remove)?;
            if remove {
                fs::remove_dir(entry.path())?;
            }
        } else {
            if metadata.is_file() {
                total += metadata.len();
            }
            if remove {
                fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(total)
}
#[cfg(target_os = "linux")]
fn scan(
    home: &Path,
    logical: &Path,
    dir: &fs::File,
    dry: bool,
    mount: u64,
    rows: &mut Vec<Value>,
) -> Result<()> {
    use std::os::fd::AsRawFd;
    let anchor = PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd()));
    for e in fs::read_dir(anchor)? {
        let e = e?;
        if !e.file_type()?.is_dir() {
            continue;
        }
        let child = opened_dir(&e.path())?;
        if mount_id(&child)? != mount {
            continue;
        }
        let path = logical.join(e.file_name());
        if safe(home, &path)
            && child.metadata()?.modified()?.elapsed().unwrap_or_default()
                > Duration::from_secs(30 * 86400)
        {
            let n = walk_contents(&child, mount, false)?;
            rows.push(json!({"path":path,"bytes":n}));
            if !dry {
                eprint!(
                    "Remove {} ({})? [y/N] ",
                    path.display(),
                    chonk_lib::bytes(n)
                );
                io::stderr().flush()?;
                let mut answer = String::new();
                io::stdin().read_line(&mut answer)?;
                if matches!(answer.trim(), "y" | "Y") {
                    walk_contents(&child, mount, true)?;
                    fs::remove_dir(e.path())?
                }
            }
        } else {
            scan(home, &path, &child, dry, mount, rows)?
        }
    }
    Ok(())
}
fn dev_scan(dry: bool) -> Result<Vec<Value>> {
    #[cfg(target_os = "linux")]
    {
        let home = PathBuf::from(std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME missing"))?);
        ensure!(
            home.is_absolute() && home.parent().is_some() && fs::canonicalize(&home)? == home,
            "Unsafe HOME"
        );
        let mut dir = opened_dir(Path::new("/"))?;
        use std::os::fd::AsRawFd;
        for c in home.components() {
            if let Component::Normal(name) = c {
                dir = opened_dir(
                    &PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd())).join(name),
                )?
            }
        }
        let mut rows = Vec::new();
        if !dry {
            restrict_removal(&dir)?;
        }
        scan(&home, &home, &dir, dry, mount_id(&dir)?, &mut rows)?;
        Ok(rows)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dry;
        Err(anyhow!("Requires Linux"))
    }
}
fn release_field<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name == key).then(|| value.trim().trim_matches(['\'', '"']))
    })
}

fn execute(cmd: Cmd) -> Result<(Value, bool)> {
    match cmd {
        Cmd::Apply { config, dry_run } => Ok((session::apply(config.as_deref(), dry_run)?, true)),
        Cmd::Mode {
            name,
            dark,
            dry_run,
        } => Ok((session::mode(&name, dark, dry_run)?, true)),
        Cmd::Mood { .. } => Ok((dashboard::mood(), true)),
        Cmd::Card { .. } => Ok((card::collect()?, true)),
        Cmd::Try { .. } => unreachable!(),
        Cmd::Profile { profile, dark } => Ok((profile::execute(profile, dark)?, true)),
        Cmd::Doctor => {
            let id = fs::read_to_string("/etc/os-release")
                .unwrap_or_default()
                .lines()
                .any(|l| {
                    l.strip_prefix("ID=")
                        .is_some_and(|v| v.trim_matches(['\'', '"']) == "chonkos")
                });
            let disk = output("df", &["-P", "/"]).ok().and_then(|s| {
                s.lines()
                    .nth(1)?
                    .split_whitespace()
                    .nth(4)?
                    .trim_end_matches('%')
                    .parse::<u32>()
                    .ok()
            });
            let failed = output("systemctl", &["--failed", "--no-legend"]).ok();
            let sources = Path::new("/etc/apt/sources.list.d/chonkos.sources").is_file();
            let key = [
                "/usr/share/keyrings/chonkos.gpg",
                "/usr/share/keyrings/chonkos-archive-keyring.gpg",
            ]
            .iter()
            .any(|p| Path::new(p).is_file());
            let system = if Path::new("/etc/sysctl.d/90-chonkos.conf").is_file() {
                "ok"
            } else {
                "warn"
            };
            let parent_debian = fs::read_to_string("/etc/dpkg/origins/chonkos")
                .unwrap_or_default()
                .lines()
                .any(|l| {
                    l.strip_prefix("Parent:")
                        .is_some_and(|s| s.trim() == "Debian")
                });
            let id_like_debian = fs::read_to_string("/usr/lib/os-release")
                .ok()
                .is_some_and(|s| {
                    release_field(&s, "ID_LIKE")
                        .is_some_and(|v| v.split_whitespace().any(|id| id == "debian"))
                });
            // zram-tools activates swap with swapon(8), so no dev-zram0.swap unit
            // exists; fall back to the unit checks for other backends.
            let swap = chonk_lib::metrics::zram_active().unwrap_or(false)
                || chonk_lib::unit_active("dev-zram0.swap")
                || chonk_lib::unit_active("zramswap.service");
            let firewall_warning = (chonk_lib::unit_active("ufw.service")
                && chonk_lib::unit_active("nftables.service"))
            .then_some("ufw and nftables are both active; Netfilter policies may conflict");
            let (config, nvidia, valid) = session::doctor();
            let ok = valid
                && id
                && parent_debian
                && id_like_debian
                && sources
                && key
                && swap
                && disk.is_some()
                && failed.as_ref().is_some_and(|s| s.trim().is_empty());
            Ok((
                json!({"chonk_toml":config,"nvidia_driver":nvidia,"timestamp":chonk_lib::utc_now()?,"chonkos":id,"chonkos-system":system,"apt_sources":sources,"keyring":key,"dpkg_parent_debian":parent_debian,"id_like_debian":id_like_debian,"zram_active":swap,"firewall_warning":firewall_warning,"root_used_percent":disk,"disk_warning":disk.is_some_and(|v|v>85),"failed_units":failed}),
                ok,
            ))
        }
        Cmd::Update { yes } => {
            ensure!(root(), "Run with: sudo chonk update");
            run("apt-get", &["update"])?;
            let count = output("apt-get", &["-s", "upgrade"])?
                .lines()
                .filter(|l| l.starts_with("Inst "))
                .count();
            eprintln!("Upgrades: {count}");
            for op in ["upgrade", "autoremove"] {
                let mut args = vec![op];
                if yes {
                    args.push("-y")
                }
                run("apt-get", &args)?
            }
            Ok((
                json!({"upgrades":count,"reboot_required":Path::new("/var/run/reboot-required").exists()}),
                true,
            ))
        }
        Cmd::Clean { dry_run, dev } => {
            if !dry_run {
                ensure!(root(), "Run with: sudo chonk clean");
                run("apt-get", &["clean"])?;
                run("apt-get", &["autoremove"])?;
                run("journalctl", &["--vacuum-size=50M"])?
            }
            Ok((
                json!({"dry_run":dry_run,"dev_directories":if dev{dev_scan(dry_run)?}else{vec![]}}),
                true,
            ))
        }
        Cmd::Docs { tool } => {
            let fallback = include_str!("../../../../docs/TOOLS.md");
            let doc = fs::read_to_string("/usr/share/doc/chonkos-tools/TOOLS.md")
                .unwrap_or_else(|_| fallback.into());
            let body = if let Some(name) = tool {
                let mut lines = doc
                    .lines()
                    .skip_while(|l| l.strip_prefix("## ") != Some(name.as_str()));
                let first = lines
                    .next()
                    .ok_or_else(|| anyhow!("Unknown tool: {name}"))?;
                let depth = first.bytes().take_while(|b| *b == b'#').count();
                std::iter::once(first)
                    .chain(lines.take_while(|l| {
                        !l.starts_with('#') || l.bytes().take_while(|b| *b == b'#').count() > depth
                    }))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                doc
            };
            Ok((json!({"docs":body}), true))
        }
    }
}
fn main() {
    let cli = Cli::parse();
    let tty = matches!(cli.command, Cmd::Card { tty: true, .. });
    let machine = cli.json || matches!(cli.command, Cmd::Mood { waybar: true });
    let plain = match cli.command {
        Cmd::Try { label, command } => {
            let result = trial::run(label, command, cli.json);
            let code = match result {
                Ok((_, code)) if !cli.json => code,
                result => chonk_lib::report(result, cli.json),
            };
            std::process::exit(code);
        }
        Cmd::Card { plain, .. } => Some(plain),
        _ => None,
    };
    let (value, ok) =
        execute(cli.command).unwrap_or_else(|e| (json!({"error":format!("{e:#}")}), false));
    if machine {
        println!("{value}")
    } else if let Some(doc) = value["docs"].as_str() {
        println!("{doc}")
    } else if let Some(err) = value["error"].as_str() {
        eprintln!("{err}")
    } else if tty {
        dashboard::tty(&value)
    } else if let Some(plain) = plain {
        card::render(&value, plain)
    } else if let Some(obj) = value.as_object() {
        if let Some(warnings) = value["warnings"].as_array() {
            for warning in warnings.iter().filter_map(Value::as_str) {
                eprintln!("Warning: {warning}");
            }
        }
        chonk_lib::table(
            &obj.iter()
                .filter(|(k, _)| k.as_str() != "warnings")
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string()),
                    )
                })
                .collect::<Vec<_>>(),
        )
    }
    std::process::exit(i32::from(!ok));
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn confines_deletion_after_rename_and_skips_symlinks() -> Result<()> {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!("chonk-clean-{}", std::process::id()));
        fs::create_dir(&base)?;
        let home = base.join("home");
        fs::create_dir(&home)?;
        let target = home.join("target");
        fs::create_dir(&target)?;
        let sentinel = base.join("sentinel");
        fs::write(&sentinel, "keep")?;
        symlink(&base, target.join("outside"))?;
        let dir = opened_dir(&target)?;
        let mount = mount_id(&dir)?;
        let home_fd = opened_dir(&home)?;
        std::thread::spawn(move || -> Result<()> {
            restrict_removal(&home_fd)?;
            walk_contents(&dir, mount, true)?;
            ensure!(sentinel.is_file());
            fs::write(target.join("keep"), "keep")?;
            Ok(())
        })
        .join()
        .unwrap()?;
        let dir = opened_dir(&home.join("target"))?;
        fs::rename(home.join("target"), base.join("moved"))?;
        let home_fd = opened_dir(&home)?;
        std::thread::spawn(move || -> Result<()> {
            restrict_removal(&home_fd)?;
            ensure!(walk_contents(&dir, mount, true).is_err());
            Ok(())
        })
        .join()
        .unwrap()?;
        assert!(base.join("moved/keep").is_file());
        fs::remove_dir_all(base)?;
        Ok(())
    }
    #[test]
    fn docs_by_tool() {
        for name in ["chonk", "scale", "nap", "whisker", "seal"] {
            let (value, ok) = execute(Cmd::Docs {
                tool: Some(name.into()),
            })
            .unwrap();
            let body = value["docs"].as_str().unwrap();
            assert!(ok);
            assert_eq!(body.lines().next(), Some(format!("## {name}").as_str()));
            assert_eq!(body.lines().filter(|l| l.starts_with("## ")).count(), 1);
        }
        assert!(execute(Cmd::Docs {
            tool: Some("ram".into())
        })
        .is_err());
    }
    #[test]
    fn paths() {
        let h = Path::new("home");
        assert!(safe(h, Path::new("home/work/target")));
        assert!(!safe(h, Path::new("home/../target")));
        assert!(!safe(h, Path::new("else/node_modules")));
    }
}
