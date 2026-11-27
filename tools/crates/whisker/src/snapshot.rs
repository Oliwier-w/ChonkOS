use crate::procnet::{parse, Socket};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

const BASELINE: &str = "var/lib/chonkos/whisker-baseline.json";

pub(super) fn sockets(root_dir: &Path, all: bool) -> Result<Vec<Socket>> {
    let mut rows = Vec::new();
    for proto in ["tcp", "tcp6", "udp", "udp6"] {
        match fs::read_to_string(root_dir.join("proc/net").join(proto)) {
            Ok(data) => rows.extend(parse(&data, proto, all)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(rows)
}

type SocketOwners = BTreeMap<u64, BTreeSet<(u32, String)>>;

pub(super) fn owners(
    root_dir: &Path,
    wanted: &BTreeSet<u64>,
    identity: impl Fn(&Path) -> Option<String>,
) -> Result<(SocketOwners, bool)> {
    let mut map = SocketOwners::new();
    let mut hidden = false;
    if wanted.is_empty() {
        return Ok((map, hidden));
    }
    for entry in fs::read_dir(root_dir.join("proc"))?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let path = entry.path();
        let fds = match fs::read_dir(path.join("fd")) {
            Ok(fds) => fds,
            Err(error) => {
                hidden |= error.kind() == io::ErrorKind::PermissionDenied;
                continue;
            }
        };
        let mut hits = BTreeSet::new();
        for fd in fds.flatten() {
            match fs::read_link(fd.path()) {
                Ok(link) => {
                    let inode = link.to_str().and_then(|link| {
                        link.strip_prefix("socket:[")?
                            .strip_suffix(']')?
                            .parse()
                            .ok()
                    });
                    hits.extend(inode.filter(|inode| wanted.contains(inode)));
                }
                Err(error) => hidden |= error.kind() == io::ErrorKind::PermissionDenied,
            }
        }
        if !hits.is_empty() {
            if let Some(name) = identity(&path) {
                for inode in hits {
                    map.entry(inode).or_default().insert((pid, name.clone()));
                }
            }
        }
    }
    Ok((map, hidden))
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Scope {
    Loopback,
    Exposed,
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
struct Entry {
    proto: String,
    bind_addr: String,
    port: u16,
    scope: Scope,
    exe_path: String,
}

#[derive(Deserialize, Serialize)]
struct Baseline {
    schema_version: u32,
    timestamp: String,
    sha256: String,
    entries: Vec<Entry>,
}

fn snapshot(root_dir: &Path) -> Result<Vec<Entry>> {
    let sockets = sockets(root_dir, false)?;
    let (owners, _) = owners(
        root_dir,
        &sockets.iter().map(|socket| socket.inode).collect(),
        |path| {
            Some(fs::read_link(path.join("exe")).map_or_else(
                |_| "unknown".into(),
                |exe| exe.to_string_lossy().into_owned(),
            ))
        },
    )?;
    let mut entries = BTreeSet::new();
    for socket in sockets {
        let ip = socket.addr.ip();
        let scope = if ip.is_loopback()
            || matches!(ip, std::net::IpAddr::V6(addr) if addr.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()))
        {
            Scope::Loopback
        } else {
            Scope::Exposed
        };
        let paths = owners
            .get(&socket.inode)
            .into_iter()
            .flatten()
            .map(|(_, path)| path.as_str());
        for exe_path in paths.chain((!owners.contains_key(&socket.inode)).then_some("unknown")) {
            entries.insert(Entry {
                proto: socket.proto.clone(),
                bind_addr: ip.to_string(),
                port: socket.addr.port(),
                scope: scope.clone(),
                exe_path: exe_path.into(),
            });
        }
    }
    Ok(entries.into_iter().collect())
}

fn checksum(entries: &[Entry]) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(entries)?)
    ))
}

pub(super) fn baseline(root_dir: &Path, save: bool) -> Result<Value> {
    ensure!(!save || chonk_lib::root_at(root_dir), "Run with sudo");
    let entries = snapshot(root_dir)?;
    let baseline = Baseline {
        schema_version: 1,
        timestamp: chonk_lib::utc_now()?,
        sha256: checksum(&entries)?,
        entries,
    };
    if save {
        chonk_lib::write_public_config(
            &root_dir.join(BASELINE),
            &serde_json::to_string(&baseline)?,
        )?;
    }
    let mut value = serde_json::to_value(baseline)?;
    value["status"] = json!(if save { "saved" } else { "snapshot" });
    Ok(value)
}

fn by_port(entries: &[Entry]) -> BTreeMap<(&str, u16), Vec<&Entry>> {
    let mut ports = BTreeMap::<_, Vec<_>>::new();
    for entry in entries {
        ports
            .entry((entry.proto.as_str(), entry.port))
            .or_default()
            .push(entry);
    }
    ports
}

fn compare(previous: &[Entry], live: &[Entry]) -> (Vec<Value>, i32) {
    let previous = by_port(previous);
    let live = by_port(live);
    let mut changes = Vec::new();
    let mut code = 0;
    for (port, entries) in &live {
        let old = previous.get(port).map_or(&[][..], Vec::as_slice);
        for entry in entries {
            if old.contains(entry) {
                continue;
            }
            let same_address = |before: &&Entry| before.bind_addr == entry.bind_addr;
            let change = if old.is_empty() {
                if entry.scope == Scope::Loopback {
                    Some(("WARNING", "new_loopback_port"))
                } else {
                    Some(("CRITICAL", "new_exposed_port"))
                }
            } else if old.iter().any(same_address) {
                Some(("CRITICAL", "executable_changed"))
            } else if entry.scope == Scope::Exposed {
                let reason = if old.iter().all(|e| e.scope == Scope::Loopback) {
                    "promoted_to_exposed"
                } else {
                    "new_exposed_port"
                };
                Some(("CRITICAL", reason))
            } else if !old.iter().any(|e| e.exe_path == entry.exe_path) {
                Some(("CRITICAL", "executable_changed"))
            } else {
                None
            };
            if let Some((severity, reason)) = change {
                code = 2;
                changes.push(json!({"severity":severity,"reason":reason,"entry":entry}));
            }
        }
    }
    for (port, entries) in previous {
        if !live.contains_key(&port) {
            for entry in entries {
                changes.push(
                    json!({"severity":"INFO","reason":"port_no_longer_listening","entry":entry}),
                );
            }
        }
    }
    (changes, code)
}

pub(super) fn drift(root_dir: &Path) -> Result<(Value, i32)> {
    let bytes = match fs::read(root_dir.join(BASELINE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((json!({"status":"uninitialized","changes":[]}), 0));
        }
        Err(error) => return Err(error.into()),
    };
    let mut saved: Baseline = serde_json::from_slice(&bytes).context("Reading whisker baseline")?;
    ensure!(
        saved.schema_version == 1,
        "Unsupported whisker baseline schema"
    );
    saved.entries.sort_unstable();
    ensure!(
        saved.sha256 == checksum(&saved.entries)?,
        "Whisker baseline checksum mismatch"
    );
    let (changes, code) = compare(&saved.entries, &snapshot(root_dir)?);
    let status = if changes.is_empty() { "clean" } else { "drift" };
    Ok((json!({"status":status,"changes":changes}), code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_listener_changes() {
        let listener = |port, addr: &str, scope, exe: &str| Entry {
            proto: "tcp".into(),
            bind_addr: addr.into(),
            port,
            scope,
            exe_path: exe.into(),
        };
        let old = [listener(
            5432,
            "127.0.0.1",
            Scope::Loopback,
            "/usr/bin/postgres",
        )];
        assert_eq!(compare(&old, &old), (vec![], 0));
        for (entry, severity, reason) in [
            (
                listener(5432, "0.0.0.0", Scope::Exposed, "/usr/bin/postgres"),
                "CRITICAL",
                "promoted_to_exposed",
            ),
            (
                listener(5432, "127.0.0.1", Scope::Loopback, "/opt/postgres"),
                "CRITICAL",
                "executable_changed",
            ),
        ] {
            let (changes, code) = compare(&old, &[entry]);
            assert_eq!(code, 2);
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0]["severity"], severity);
            assert_eq!(changes[0]["reason"], reason);
        }
        let live = [
            listener(5432, "127.0.0.1", Scope::Loopback, "/usr/bin/postgres"),
            listener(5433, "127.0.0.1", Scope::Loopback, "/opt/postgres"),
        ];
        let (changes, code) = compare(&old, &live);
        assert_eq!(code, 2);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["severity"], "WARNING");
        assert_eq!(changes[0]["entry"]["port"], 5433);
    }

    #[test]
    fn baseline_round_trip_and_drift() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path();
        assert_eq!(drift(path)?.0["status"], "uninitialized");
        fs::create_dir_all(path.join("proc/self"))?;
        fs::write(path.join("proc/self/status"), "Uid: 0 0 0 0\n")?;
        fs::create_dir_all(path.join("proc/net"))?;
        let tcp = path.join("proc/net/tcp");
        fs::write(&tcp, "0: 0100007F:0016 unused 0A 0 0 0 1000 0 42\n")?;
        let saved = baseline(path, true)?;
        assert_eq!(saved["entries"][0]["exe_path"], "unknown");
        assert_eq!(saved["entries"][0]["bind_addr"], "127.0.0.1");
        assert_eq!(drift(path)?.0["status"], "clean");
        fs::write(&tcp, "0: 00000000:0016 unused 0A 0 0 0 1000 0 42\n")?;
        let (value, code) = drift(path)?;
        assert_eq!(code, 2);
        assert_eq!(value["changes"][0]["reason"], "promoted_to_exposed");
        fs::write(tcp, "")?;
        let (value, code) = drift(path)?;
        assert_eq!(code, 0);
        assert_eq!(value["changes"][0]["severity"], "INFO");
        Ok(())
    }
}
