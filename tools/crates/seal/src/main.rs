use anyhow::{anyhow, ensure, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
};
use tempfile::NamedTempFile;

const TRACKED: &[&str] = &[
    "etc/apt/sources.list",
    "etc/apt/sources.list.d",
    "etc/default/zramswap",
    "etc/fstab",
    "etc/systemd/logind.conf.d",
    "etc/xdg/fastfetch",
    "etc/sysctl.d",
];
mod cli;
use cli::{Cli, Cmd};
#[derive(Serialize, Deserialize)]
struct Manifest {
    id: String,
    date: String,
    message: String,
    files: BTreeMap<String, String>,
    packages: Option<usize>,
}
struct Archive {
    root: PathBuf,
    store: PathBuf,
}
fn stamp() -> Result<String> {
    Ok(
        time::OffsetDateTime::now_utc().format(time::macros::format_description!(
            "[year][month][day]-[hour][minute][second]"
        ))?,
    )
}
fn checked(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
    for c in path.components() {
        ensure!(c != Component::ParentDir, "parent component forbidden");
        prefix.push(c);
        match fs::symlink_metadata(&prefix) {
            Ok(m) => ensure!(!m.is_symlink(), "symlink forbidden: {}", prefix.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn allowed(key: &str) -> bool {
    let Some(rel) = key.strip_prefix('/') else {
        return false;
    };
    if rel.contains(['\\', ':', '\0'])
        || rel.split('/').any(|p| {
            matches!(p, "" | "." | "..")
                || p.ends_with("_key")
                || p.ends_with(".pem")
                || p.ends_with(".key")
                || matches!(
                    p,
                    "shadow"
                        | "gshadow"
                        | "id_rsa"
                        | "id_dsa"
                        | "id_ecdsa"
                        | "id_ed25519"
                        | "id_ecdsa_sk"
                        | "id_ed25519_sk"
                )
        })
    {
        return false;
    }
    TRACKED
        .iter()
        .any(|t| rel == *t || rel.strip_prefix(t).is_some_and(|s| s.starts_with('/')))
}
// Keep the directory open so renaming an ancestor cannot redirect a restore.
struct Directory {
    path: PathBuf,
    #[cfg(target_os = "linux")]
    _handle: File,
}
impl Directory {
    #[cfg(target_os = "linux")]
    fn open(path: &Path, create: bool) -> Result<Self> {
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
        let mut handle = File::open("/")?;
        let anchor = |f: &File| PathBuf::from(format!("/proc/self/fd/{}", f.as_raw_fd()));
        for part in std::path::absolute(path)?.components() {
            if part == Component::RootDir {
                continue;
            }
            let Component::Normal(name) = part else {
                anyhow::bail!("invalid directory component")
            };
            let next = anchor(&handle).join(name);
            let open = || {
                File::options()
                    .read(true)
                    .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                    .open(&next)
            };
            handle = match open() {
                Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
                    match fs::create_dir(&next) {
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                        result => result?,
                    }
                    open()?
                }
                result => result?,
            };
        }
        Ok(Self {
            path: anchor(&handle),
            _handle: handle,
        })
    }
    #[cfg(not(target_os = "linux"))]
    fn open(path: &Path, create: bool) -> Result<Self> {
        checked(path)?;
        if create {
            fs::create_dir_all(path)?;
        }
        Ok(Self { path: path.into() })
    }
}
fn read_regular(p: &Path) -> Result<File> {
    let dir = Directory::open(p.parent().context("missing parent")?, false)?;
    open_regular(&dir.path.join(p.file_name().context("missing file name")?))
}
fn open_regular(p: &Path) -> Result<File> {
    ensure!(
        fs::symlink_metadata(p)?.is_file(),
        "expected regular file: {}",
        p.display()
    );
    let mut opts = File::options();
    opts.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = opts.open(p)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "expected regular file");
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(metadata.nlink() == 1, "hard-linked file forbidden");
    }
    Ok(file)
}
fn read_text(p: &Path) -> Result<String> {
    let mut s = String::new();
    read_regular(p)?.read_to_string(&mut s)?;
    Ok(s)
}
fn transfer(p: &Path, mut dest: impl Write) -> Result<String> {
    let mut f = BufReader::new(read_regular(p)?);
    let mut sha = Sha256::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        // Bound memory use and reject lines too long to inspect as configuration.
        if f.by_ref().take(1024 * 1024).read_until(b'\n', &mut line)? == 0 {
            break;
        }
        ensure!(line.len() < 1024 * 1024, "configuration line too long");
        let contains = |marker: &[u8]| line.windows(marker.len()).any(|bytes| bytes == marker);
        ensure!(
            !(contains(b"-----BEGIN ") && contains(b"PRIVATE KEY"))
                && !contains(b"PuTTY-User-Key-File-"),
            "private key material forbidden"
        );
        sha.update(&line);
        dest.write_all(&line)?;
    }
    Ok(format!("{:x}", sha.finalize()))
}
impl Archive {
    fn scan(&self, out: Option<&Path>) -> Result<BTreeMap<String, String>> {
        let mut files = BTreeMap::new();
        for t in TRACKED {
            self.walk(&format!("/{t}"), out, &mut files)?
        }
        Ok(files)
    }
    fn walk(
        &self,
        key: &str,
        out: Option<&Path>,
        files: &mut BTreeMap<String, String>,
    ) -> Result<()> {
        if !allowed(key) {
            return Ok(());
        }
        let rel = &key[1..];
        let src = self.root.join(rel);
        if src == self.store {
            return Ok(());
        }
        for ancestor in src.ancestors().collect::<Vec<_>>().into_iter().rev() {
            match fs::symlink_metadata(ancestor) {
                Ok(m) if m.is_symlink() => return Ok(()),
                Ok(_) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
        let m = fs::symlink_metadata(&src)?;
        if m.is_dir() {
            let dir = Directory::open(&src, false)?;
            for e in fs::read_dir(&dir.path)? {
                let e = e?;
                let name = e.file_name();
                let name = name.to_str().context("non UTF-8 file name")?;
                self.walk(&format!("{key}/{name}"), out, files)?
            }
        } else if m.is_file() {
            let hash = if let Some(base) = out {
                let p = base.join(rel);
                fs::create_dir_all(p.parent().unwrap())?;
                transfer(&src, File::create_new(p)?)?
            } else {
                transfer(&src, io::sink())?
            };
            files.insert(key.into(), hash);
        }
        Ok(())
    }
    fn mark(&self, message: String, packages: Option<BTreeSet<String>>) -> Result<Manifest> {
        let store = Directory::open(&self.store, true)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = store._handle.metadata()?;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o022 == 0,
                "snapshot store must be owned by the current user and writable only by its owner"
            );
        }
        let date = stamp()?;
        let slug: String = message
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(48)
            .flat_map(char::to_lowercase)
            .collect();
        let id = format!("{date}-{}", if slug.is_empty() { "seal" } else { &slug });
        let tmp = tempfile::Builder::new()
            .prefix(".pending-")
            .tempdir_in(&store.path)?;
        let dir = tmp.path().join("files");
        fs::create_dir(&dir)?;
        let files = self.scan(Some(&dir))?;
        if let Some(pkgs) = &packages {
            let mut f = File::create_new(tmp.path().join("packages.txt"))?;
            for p in pkgs {
                writeln!(f, "{p}")?
            }
        }
        let m = Manifest {
            id: id.clone(),
            date,
            message,
            files,
            packages: packages.map(|p| p.len()),
        };
        serde_json::to_writer(File::create_new(tmp.path().join("manifest.json"))?, &m)?;
        let dest = store.path.join(id);
        ensure!(
            fs::symlink_metadata(&dest).is_err_and(|e| e.kind() == io::ErrorKind::NotFound),
            "snapshot destination is unavailable"
        );
        fs::rename(tmp.path(), dest)?;
        Ok(m)
    }
    fn load(&self, id: &str) -> Result<Manifest> {
        ensure!(
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "invalid snapshot ID"
        );
        let m: Manifest =
            serde_json::from_reader(read_regular(&self.store.join(id).join("manifest.json"))?)?;
        ensure!(m.id == id, "snapshot ID mismatch");
        Ok(m)
    }
    fn list(&self) -> Result<Vec<Manifest>> {
        checked(&self.store)?;
        let mut result = vec![];
        if !self.store.try_exists()? {
            return Ok(result);
        }
        for e in fs::read_dir(&self.store)? {
            let e = e?;
            let id = e.file_name();
            if !e.file_type()?.is_dir() || id.to_string_lossy().starts_with('.') {
                continue;
            }
            if e.path().join("manifest.json").try_exists()? {
                result.push(self.load(id.to_str().context("invalid snapshot ID")?)?)
            }
        }
        result.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(result)
    }
    fn diff(&self, m: &Manifest, pkgs: Option<BTreeSet<String>>) -> Result<Value> {
        let current = self.scan(None)?;
        let packages = if let (Some(_), Some(now)) = (m.packages, pkgs) {
            let old: BTreeSet<String> = read_text(&self.store.join(&m.id).join("packages.txt"))?
                .lines()
                .map(str::to_owned)
                .collect();
            json!({"added":now.difference(&old).collect::<Vec<_>>(),"removed":old.difference(&now).collect::<Vec<_>>(),"modified":[]})
        } else {
            Value::Null
        };
        Ok(
            json!({"id":m.id,"files":{"modified":current.iter().filter(|(p,h)|m.files.get(*p).is_some_and(|v|v!=*h)).map(|(p,_)|p).collect::<Vec<_>>(),"removed":m.files.keys().filter(|p|!current.contains_key(*p)).collect::<Vec<_>>(),"added":current.keys().filter(|p|!m.files.contains_key(*p)).collect::<Vec<_>>()},"packages":packages}),
        )
    }
    fn restore(&self, m: &Manifest, key: &str, dry: bool) -> Result<Value> {
        ensure!(allowed(key), "forbidden target path");
        let hash = m.files.get(key).context("target absent from manifest")?;
        let rel = &key[1..];
        let src = self.store.join(&m.id).join("files").join(rel);
        let dst = self.root.join(rel);
        checked(&dst)?;
        let exists = dst.try_exists()?;
        if exists {
            ensure!(
                fs::symlink_metadata(&dst)?.is_file(),
                "target is not a regular file"
            )
        }
        let backup = if exists {
            Some(dst.with_file_name(format!(
                "{}.bak.{}",
                dst.file_name().unwrap().to_string_lossy(),
                stamp()?
            )))
        } else {
            None
        };
        let result = json!({"target":key,"backup":backup,"dry_run":dry});
        if dry {
            ensure!(
                &transfer(&src, io::sink())? == hash,
                "snapshot checksum mismatch"
            );
            return Ok(result);
        }
        let parent = Directory::open(dst.parent().unwrap(), true)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            // Serialize restores to the same directory through backup and replacement.
            if unsafe { libc::flock(parent._handle.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(io::Error::last_os_error().into());
            }
        }
        let dst = parent.path.join(dst.file_name().unwrap());
        let mut tmp = NamedTempFile::new_in(&parent.path)?;
        ensure!(
            &transfer(&src, &mut tmp)? == hash,
            "snapshot checksum mismatch"
        );
        if let Some(bak) = backup {
            let mut old = open_regular(&dst)?;
            let original = old.metadata()?;
            let mut copy = NamedTempFile::new_in(&parent.path)?;
            io::copy(&mut old, &mut copy)?;
            copy.as_file().sync_all()?;
            copy.persist_noclobber(parent.path.join(bak.file_name().unwrap()))?;
            #[cfg(target_os = "linux")]
            parent._handle.sync_all()?;
            tmp.as_file().set_permissions(original.permissions())?;
            #[cfg(target_os = "linux")]
            {
                use std::os::unix::fs::MetadataExt;
                let current = fs::symlink_metadata(&dst)?;
                ensure!(
                    (original.dev(), original.ino()) == (current.dev(), current.ino()),
                    "restore target changed"
                );
            }
        }
        tmp.as_file().sync_all()?;
        if exists {
            ensure!(
                fs::symlink_metadata(&dst)?.is_file(),
                "restore target changed"
            );
            tmp.persist(dst)?;
        } else {
            tmp.persist_noclobber(dst)?;
        }
        #[cfg(target_os = "linux")]
        parent._handle.sync_all()?;
        Ok(result)
    }
}
#[cfg(target_os = "linux")]
fn packages() -> Result<Option<BTreeSet<String>>> {
    use std::process::Command;
    match Command::new("apt-mark").arg("showmanual").output() {
        Ok(o) => {
            ensure!(o.status.success(), "apt-mark showmanual failed");
            Ok(Some(
                String::from_utf8(o.stdout)?
                    .lines()
                    .map(str::to_owned)
                    .collect(),
            ))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
#[cfg(not(target_os = "linux"))]
fn packages() -> Result<Option<BTreeSet<String>>> {
    Ok(None)
}
fn run(cli: Cli) -> Result<Value> {
    let a = Archive {
        root: "/".into(),
        store: std::path::absolute(cli.store)?,
    };
    match cli.cmd {
        Cmd::Mark { message } => Ok(json!({"id":a.mark(message,packages()?)?.id})),
        Cmd::List => {
            let rows=a.list()?.into_iter().map(|m|json!({"id":m.id,"date":m.date,"message":m.message,"files":m.files.len(),"packages":m.packages})).collect::<Vec<_>>();
            if !cli.json {
                let mut table = vec![("ID".into(), "Date  Description  Files  Packages".into())];
                for r in rows {
                    table.push((
                        r["id"].as_str().unwrap().into(),
                        format!(
                            "{}  {}  {}  {}",
                            r["date"].as_str().unwrap(),
                            r["message"].as_str().unwrap(),
                            r["files"],
                            r["packages"]
                        ),
                    ))
                }
                chonk_lib::table(&table);
                return Ok(Value::Null);
            }
            Ok(json!(rows))
        }
        Cmd::Diff { id } => {
            let m = match id {
                Some(s) => a.load(&s)?,
                None => a
                    .list()?
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow!("no snapshots"))?,
            };
            a.diff(&m, packages()?)
        }
        Cmd::Restore {
            id,
            target_path,
            dry_run,
        } => {
            let manifest = a.load(&id)?;
            if let Some(path) = target_path {
                a.restore(&manifest, &path, dry_run)
            } else {
                let files = manifest
                    .files
                    .keys()
                    .map(|path| a.restore(&manifest, path, true))
                    .collect::<Result<Vec<_>>>()?;
                Ok(json!({"id":id,"dry_run":true,"files":files}))
            }
        }
        Cmd::Export { file } => {
            let os = fs::read_to_string(a.root.join("etc/os-release"))?;
            let fields: BTreeMap<_, _> = os
                .lines()
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k, v.trim_matches(['"', '\''])))
                .collect();
            let v = json!({"manual":packages()?,"os":{"name":fields.get("NAME"),"version":fields.get("VERSION_ID")},"files":a.scan(None)?});
            if let Some(p) = file {
                checked(&p)?;
                let mut f = NamedTempFile::new_in(std::path::absolute(&p)?.parent().unwrap())?;
                serde_json::to_writer(&mut f, &v)?;
                f.persist(p)?;
            } else {
                println!("{v}")
            }
            Ok(Value::Null)
        }
    }
}
fn main() {
    let cli = Cli::parse();
    let machine = cli.json;
    let result = run(cli);
    if matches!(result, Ok(Value::Null)) {
        return;
    }
    std::process::exit(chonk_lib::report(result.map(|v| (v, 0)), machine))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_private_key_content() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("config");
        fs::write(&path, b"-----BEGIN OPENSSH PRIVATE KEY-----\n")?;
        assert!(transfer(&path, io::sink()).is_err());
        Ok(())
    }
    #[test]
    fn lifecycle() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let a = Archive {
            root: temp.path().join("root"),
            store: temp.path().join("store"),
        };
        let dir = a.root.join("etc/sysctl.d");
        fs::create_dir_all(&dir)?;
        for name in ["config", "removed", "id_key", "secret.pem", "secret.key"] {
            fs::write(dir.join(name), "old")?
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("config"), dir.join("link"))?;
        let p = |s: &str| Some(BTreeSet::from([s.to_owned()]));
        let m = a.mark("first".into(), p("one"))?;
        let m = a.load(&m.id)?;
        assert_eq!(m.files.len(), 2);
        fs::write(dir.join("config"), "new")?;
        fs::write(dir.join("added"), "new")?;
        fs::remove_file(dir.join("removed"))?;
        let d = a.diff(&m, p("two"))?;
        for (kind, name) in [
            ("modified", "config"),
            ("added", "added"),
            ("removed", "removed"),
        ] {
            assert_eq!(d["files"][kind], json!([format!("/etc/sysctl.d/{name}")]))
        }
        assert_eq!(d["packages"]["removed"], json!(["one"]));
        for key in [
            "/etc/sysctl.d/../shadow",
            "/etc/shadow",
            "/etc/sudoers",
            "/etc/gshadow",
            "/etc/sysctl.d/secret.key",
            "/etc/sysctl.d/added",
        ] {
            assert!(a.restore(&m, key, false).is_err())
        }
        assert!(a.load("../store").is_err());
        let key = "/etc/sysctl.d/config";
        a.restore(&m, key, true)?;
        assert_eq!(fs::read(dir.join("config"))?, b"new");
        let v = a.restore(&m, key, false)?;
        assert_eq!(fs::read(v["backup"].as_str().unwrap())?, b"new");
        assert_eq!(fs::read(dir.join("config"))?, b"old");
        #[cfg(unix)]
        {
            fs::remove_file(dir.join("config"))?;
            std::os::unix::fs::symlink(dir.join("added"), dir.join("config"))?;
            assert!(a.restore(&m, key, false).is_err());
        }
        Ok(())
    }
}
