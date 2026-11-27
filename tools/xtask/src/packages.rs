use super::normalize_text;
use anyhow::{ensure, Context, Result};
use flate2::{Compression, GzBuilder};
use md5::Md5;
use serde_json::{json, Map};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_BYTES: u64 = 1_048_576;
const MAINTAINER: &str = "Oliwier-w <oliwier-w@users.noreply.github.com>";

pub fn stage(out: &Path, bin_dir: &Path) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut binaries = Vec::new();
    let mut metrics = Map::new();
    for (name, _) in super::TOOLS {
        let path = bin_dir.join(name);
        let size = fs::metadata(&path)
            .with_context(|| format!("Reading {}", path.display()))?
            .len();
        ensure!(
            size <= MAX_BYTES,
            "{name}: {size} bytes exceeds {MAX_BYTES}"
        );
        let data = fs::read(&path)?;
        let size = data.len() as u64;
        ensure!(
            size <= MAX_BYTES,
            "{name}: {size} bytes exceeds {MAX_BYTES}"
        );
        metrics.insert(
            name.into(),
            json!({
                "size_bytes": size,
                "sha256": format!("{:x}", Sha256::digest(&data)),
            }),
        );
        binaries.push((name, data));
    }

    fs::create_dir_all(out)?;
    let work = tempfile::tempdir_in(out)?;
    let mut outputs = vec![PathBuf::from("metrics.json")];
    let mut packages = Map::new();
    for source in children(&root.join("packages"))? {
        if !source.is_dir() {
            continue;
        }
        let name = source
            .file_name()
            .and_then(OsStr::to_str)
            .context("Invalid package name")?;
        let destination = work.path().join(name);
        copy_tree(&source, &destination)?;
        if name == "chonkos-tools" {
            install_tools(&root, &destination, &binaries)?;
        }
        let installed_size = finish_package(&root, &destination, name)?;
        packages.insert(name.into(), json!({"installed_size_kib": installed_size}));
        outputs.push(name.into());
    }
    fs::write(
        work.path().join("metrics.json"),
        serde_json::to_vec_pretty(&json!({
            "version": VERSION,
            "binaries": metrics,
            "packages": packages,
        }))?,
    )?;
    for name in &outputs {
        let path = out.join(name);
        ensure!(
            !path.exists() && !path.is_symlink(),
            "{} already exists",
            path.display()
        );
    }
    for name in outputs {
        fs::rename(work.path().join(&name), out.join(name))?;
    }
    Ok(())
}

fn children(path: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort();
    Ok(entries)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    set_mode(destination, 0o755)?;
    for path in children(source)? {
        let kind = fs::symlink_metadata(&path)?.file_type();
        let mut target = destination.join(path.file_name().context("Missing file name")?);
        if kind.is_dir() {
            copy_tree(&path, &target)?;
        } else if kind.is_file() {
            let template = path.extension() == Some(OsStr::new("in"));
            if template {
                target.set_extension("");
            }
            // Scripts under usr/libexec/chonkos are started directly by
            // systemd units, so they must stay executable in the package.
            let executable = destination.ends_with("etc/update-motd.d")
                || destination.ends_with("usr/libexec/chonkos")
                || (destination.ends_with("DEBIAN")
                    && matches!(
                        target.file_name().and_then(OsStr::to_str),
                        Some("preinst" | "postinst" | "prerm" | "postrm" | "config")
                    ));
            if template
                || executable
                || path.extension() == Some(OsStr::new("sh"))
                || target.ends_with("DEBIAN/conffiles")
            {
                let mut text = normalize_text(&fs::read_to_string(&path)?);
                if template {
                    text = text.replace("{{VERSION}}", VERSION);
                }
                fs::write(&target, text)?;
            } else {
                fs::copy(&path, &target)?;
            }
            set_mode(&target, if executable { 0o755 } else { 0o644 })?;
        } else {
            anyhow::bail!("Unsupported package entry {}", path.display());
        }
    }
    Ok(())
}

fn install_tools(root: &Path, package: &Path, binaries: &[(&str, Vec<u8>)]) -> Result<()> {
    let bin = package.join("usr/bin");
    let man = package.join("usr/share/man/man1");
    fs::create_dir_all(&bin)?;
    fs::create_dir_all(&man)?;
    for (name, data) in binaries {
        let path = bin.join(name);
        fs::write(&path, data)?;
        set_mode(&path, 0o755)?;
    }
    for path in children(&root.join("docs/man"))? {
        if path.extension() == Some(OsStr::new("1")) {
            let mut target = man.join(path.file_name().context("Missing manpage name")?);
            target.set_extension("1.gz");
            gzip_text(&target, &fs::read_to_string(path)?)?;
        }
    }
    let doc = package.join("usr/share/doc/chonkos-tools");
    fs::create_dir_all(&doc)?;
    fs::write(
        doc.join("TOOLS.md"),
        normalize_text(&fs::read_to_string(root.join("docs/TOOLS.md"))?),
    )?;
    super::completions(package, true)
}

fn finish_package(root: &Path, package: &Path, name: &str) -> Result<u64> {
    let doc = package.join("usr/share/doc").join(name);
    fs::create_dir_all(&doc)?;
    let license = normalize_text(&fs::read_to_string(root.join("LICENSE"))?);
    fs::write(doc.join("copyright"), license)?;

    // Keep the release entry stable across rebuilds.
    let changelog = format!(
        "{name} ({VERSION}) trixie; urgency=medium\n\n  * ChonkOS release.\n\n -- {MAINTAINER}  Thu, 26 Nov 2026 00:00:00 +0000\n"
    );
    gzip_text(&doc.join("changelog.Debian.gz"), &changelog)?;

    let debian = package.join("DEBIAN");
    let conffiles = debian.join("conffiles");
    let conffiles = if conffiles.exists() {
        fs::read_to_string(conffiles)?
    } else {
        String::new()
    };
    let conffiles = conffiles
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .map(|path| path.trim_start_matches('/'))
        .collect::<HashSet<_>>();
    let mut sums = String::new();
    let installed_size = payload_metadata(package, package, &conffiles, &mut sums)?;
    fs::write(debian.join("md5sums"), sums)?;
    let control = debian.join("control");
    let contents = fs::read_to_string(&control)?;
    let mut updated = String::new();
    for line in contents.lines() {
        if line.starts_with("Description:") {
            updated.push_str(&format!("Installed-Size: {installed_size}\n"));
        }
        if !line.starts_with("Installed-Size:") {
            updated.push_str(line);
            updated.push('\n');
        }
    }
    fs::write(control, updated)?;
    Ok(installed_size)
}

fn payload_metadata(
    package: &Path,
    directory: &Path,
    conffiles: &HashSet<&str>,
    sums: &mut String,
) -> Result<u64> {
    let mut kib = 1;
    for path in children(directory)? {
        let relative = path.strip_prefix(package)?;
        if relative == Path::new("DEBIAN") {
            continue;
        }
        let metadata = fs::metadata(&path)?;
        if metadata.is_dir() {
            kib += payload_metadata(package, &path, conffiles, sums)?;
        } else {
            kib += metadata.len().div_ceil(1024);
            let name = relative
                .to_str()
                .context("Invalid payload path")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if !conffiles.contains(name.as_str()) {
                let mut digest = Md5::new();
                std::io::copy(&mut File::open(&path)?, &mut digest)?;
                sums.push_str(&format!("{:x}  {name}\n", digest.finalize()));
            }
        }
    }
    Ok(kib)
}

fn gzip_text(path: &Path, text: &str) -> Result<()> {
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(File::create(path)?, Compression::best());
    encoder.write_all(normalize_text(text).as_bytes())?;
    encoder.finish()?;
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}
