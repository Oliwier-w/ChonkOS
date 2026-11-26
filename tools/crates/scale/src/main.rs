use anyhow::{ensure, Result};
use chonk_lib::{bytes, metrics, output, size};
use clap::Parser;
use serde_json::json;
use std::{collections::HashSet, fs};
mod cli;
use cli::{Cli, Cmd};
fn pkg(s: &str) -> Option<(u64, &str)> {
    let (n, name) = s.split_once('\t')?;
    Some((n.parse::<u64>().ok()?.checked_mul(1024)?, name))
}
fn duration(s: &str) -> Option<u64> {
    for (unit, mult) in [
        ("min", 60000.),
        ("ms", 1.),
        ("us", 0.001),
        ("Ă‚Âµs", 0.001),
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
            for (p, n) in metrics::processes() {
                if let Ok(name) = fs::read_to_string(p.join("comm")) {
                    rows.push((
                        n,
                        format!(
                            "{} {}",
                            p.file_name().unwrap_or_default().to_string_lossy(),
                            name.trim_end()
                        ),
                    ))
                }
            }
            memory = metrics::zram_stats();
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
        assert_eq!(metrics::pss("Rss: 12 kB\nPss: 7 kB"), Some(7168));
        assert_eq!(metrics::zram("100 50 60 0"), Some((100, 50, 60)));
        assert_eq!(pkg("42\tpkg"), Some((43008, "pkg")));
    }
}
