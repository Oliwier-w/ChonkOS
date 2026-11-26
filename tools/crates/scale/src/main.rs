use anyhow::{ensure, Result};
use chonk_lib::{bytes, output, size};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::{collections::HashSet, fs, path::PathBuf};
#[derive(Parser)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Cmd,
}
#[derive(Subcommand)]
enum Cmd {
    Ram {
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    Boot {
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    Pkg {
        #[arg(long, default_value_t = 15)]
        top: usize,
    },
    Dir {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value_t = 15)]
        top: usize,
    },
}
fn pss(s: &str) -> Option<u64> {
    s.lines()
        .find_map(|l| l.strip_prefix("Pss:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}
fn zram(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.split_whitespace().map(str::parse);
    Some((it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?))
}
fn pkg(s: &str) -> Option<(u64, &str)> {
    let (n, name) = s.split_once('\t')?;
    Some((n.parse::<u64>().ok()?.checked_mul(1024)?, name))
}
fn duration(s: &str) -> Option<u64> {
    for (unit, mult) in [
        ("min", 60000.),
        ("ms", 1.),
        ("us", 0.001),
        ("µs", 0.001),
        ("s", 1000.),
        ("h", 3600000.),
    ] {
        if let Some(n) = s.strip_suffix(unit) {
            return Some((n.parse::<f64>().ok()? * mult) as u64);
        }
    }
    None
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut rows = Vec::new();
    let mut memory = None;
    let mut timed = false;
    let top = match cli.command {
        Cmd::Ram { top } => {
            for e in fs::read_dir("/proc")?.flatten() {
                if e.file_name().to_string_lossy().parse::<u32>().is_err() {
                    continue;
                }
                let p = e.path();
                if let (Some(n), Ok(name)) = (
                    fs::read_to_string(p.join("smaps_rollup"))
                        .ok()
                        .and_then(|s| pss(&s)),
                    fs::read_to_string(p.join("comm")),
                ) {
                    rows.push((
                        n,
                        format!("{} {}", e.file_name().to_string_lossy(), name.trim_end()),
                    ))
                }
            }
            if let Ok(s) = fs::read_to_string("/sys/block/zram0/mm_stat") {
                memory = zram(&s)
            }
            top
        }
        Cmd::Pkg { top } => {
            let marked = output("apt-mark", &["showmanual"])?;
            let manual: HashSet<_> = marked.lines().collect();
            for (n, p) in output("dpkg-query", &["-W", r"-f=${Installed-Size}\t${Package}\n"])?
                .lines()
                .filter_map(pkg)
            {
                rows.push((
                    n,
                    format!(
                        "{} [{}]",
                        p,
                        if manual.contains(p) { "manual" } else { "auto" }
                    ),
                ))
            }
            top
        }
        Cmd::Boot { top } => {
            timed = true;
            for l in output("systemd-analyze", &["blame"])?.lines() {
                let mut fields = l.split_whitespace().collect::<Vec<_>>();
                if let Some(name) = fields.pop() {
                    if let Some(n) = fields.into_iter().map(duration).sum::<Option<u64>>() {
                        rows.push((n, name.to_owned()))
                    }
                }
            }
            top
        }
        Cmd::Dir { path, top } => {
            let p = path.canonicalize()?;
            ensure!(
                !["/proc", "/sys", "/dev"].iter().any(|x| p.starts_with(x)),
                "virtual filesystem"
            );
            for e in fs::read_dir(p)? {
                let e = e?;
                if e.file_type()?.is_dir()
                    && !matches!(e.path().to_str(), Some("/proc" | "/sys" | "/dev"))
                {
                    rows.push((
                        size(&e.path())?,
                        e.file_name().to_string_lossy().into_owned(),
                    ))
                }
            }
            top
        }
    };
    rows.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    rows.truncate(top);
    let z=memory.map(|(o,c,m)|json!({"original_bytes":o,"compressed_bytes":c,"ratio":(c>0).then(||o as f64/c as f64),"saved_bytes":o.saturating_sub(m)}));
    if cli.json {
        return chonk_lib::json(&json!({"rows":rows,"unit":if timed{"ms"}else{"bytes"},"zram":z}));
    }
    chonk_lib::table(
        &rows
            .into_iter()
            .map(|(n, k)| (k, if timed { format!("{n} ms") } else { bytes(n) }))
            .collect::<Vec<_>>(),
    );
    if let Some(v) = z {
        println!("ZRAM");
        for (k, n) in v.as_object().unwrap() {
            println!("{k}  {n}")
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parsers() {
        assert_eq!(pss("Rss: 12 kB\nPss: 7 kB"), Some(7168));
        assert_eq!(zram("100 50 60 0"), Some((100, 50, 60)));
        assert_eq!(pkg("42\tpkg"), Some((43008, "pkg")));
    }
}
