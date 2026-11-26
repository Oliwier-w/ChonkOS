use anyhow::{ensure, Result};
use clap::Parser;
use serde_json::{json, Value};
use std::{
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};
mod cli;
use cli::{Cli, Cmd};
#[cfg(any(test, target_os = "linux"))]
mod procnet {
    use super::*;
    use chonk_lib::metrics::proc_ip as ip;
    use std::net::{IpAddr, SocketAddr};
    #[derive(Debug)]
    pub struct Socket {
        pub addr: SocketAddr,
        pub inode: u64,
        pub state: String,
        pub proto: String,
    }
    pub fn parse(s: &str, proto: &str, all: bool) -> Vec<Socket> {
        s.lines()
            .filter_map(|l| {
                let fields = l.split_whitespace().collect::<Vec<_>>();
                let state = *fields.get(3)?;
                if !all && state != if proto.starts_with("tcp") { "0A" } else { "07" } {
                    return None;
                }
                let (a, p) = fields.get(1)?.split_once(':')?;
                Some(Socket {
                    addr: SocketAddr::new(ip(a)?, u16::from_str_radix(p, 16).ok()?),
                    state: state.into(),
                    proto: proto.into(),
                    inode: fields.get(9)?.parse().ok()?,
                })
            })
            .collect()
    }
    pub fn gateway(s: &str) -> Option<IpAddr> {
        s.lines().find_map(|l| {
            let mut f = l.split_whitespace();
            f.next()?;
            let dest = f.next()?;
            let addr = f.next()?;
            let flags = u32::from_str_radix(f.next()?, 16).ok()?;
            if dest == "00000000" && flags & 3 == 3 {
                ip(addr)
            } else {
                None
            }
        })
    }
    pub fn guard(pid: u32, name: &str) -> Result<()> {
        ensure!(pid > 1 && pid != std::process::id(), "protected PID {pid}");
        ensure!(
            name != "sshd" && !name.starts_with("sshd-"),
            "protected process {name}"
        );
        Ok(())
    }
}
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use procnet::*;
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        io::{self, Write},
        os::fd::{AsRawFd, FromRawFd, OwnedFd},
        process::Command,
    };
    fn sockets(all: bool) -> Result<Vec<Socket>> {
        let mut rows = vec![];
        for p in ["tcp", "tcp6", "udp", "udp6"] {
            match fs::read_to_string(format!("/proc/net/{p}")) {
                Ok(s) => rows.extend(parse(&s, p, all)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(rows)
    }
    type SocketOwners = BTreeMap<u64, BTreeSet<(u32, String)>>;
    fn owners(wanted: &BTreeSet<u64>) -> Result<(SocketOwners, bool)> {
        let mut map = BTreeMap::<_, BTreeSet<_>>::new();
        let mut hidden = false;
        for e in fs::read_dir("/proc")?.flatten() {
            let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            let fds = match fs::read_dir(e.path().join("fd")) {
                Ok(d) => d,
                Err(e) => {
                    hidden |= e.kind() == io::ErrorKind::PermissionDenied;
                    continue;
                }
            };
            let mut hits = BTreeSet::new();
            for fd in fds.flatten() {
                match fs::read_link(fd.path()) {
                    Ok(link) => {
                        if let Some(n) = link
                            .to_str()
                            .and_then(|s| {
                                s.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
                            })
                            .filter(|n| wanted.contains(n))
                        {
                            hits.insert(n);
                        }
                    }
                    Err(e) => hidden |= e.kind() == io::ErrorKind::PermissionDenied,
                }
            }
            if !hits.is_empty() {
                if let Ok(name) = fs::read_to_string(e.path().join("comm")) {
                    for n in hits {
                        map.entry(n)
                            .or_default()
                            .insert((pid, name.trim_end().into()));
                    }
                }
            }
        }
        Ok((map, hidden))
    }
    pub fn inspect(all: bool, kill: Option<(u16, bool, bool)>, machine: bool) -> Result<Value> {
        let mut ss = sockets(all)?;
        if let Some((port, ..)) = kill {
            ss.retain(|s| s.addr.port() == port);
            ensure!(!ss.is_empty(), "no listeners on {port}")
        }
        let (map, hidden) = owners(&ss.iter().map(|s| s.inode).collect())?;
        let mut rows = vec![];
        let mut targets = BTreeSet::new();
        for s in ss {
            let exposure = if s.addr.ip().is_unspecified() {
                "exposed"
            } else if s.addr.ip().is_loopback() {
                "local"
            } else {
                "interface"
            };
            let owners = map.get(&s.inode);
            let mut row = json!({"protocol":s.proto,"address":s.addr,"state":s.state,"exposure":exposure,"pid":"-","process":"-"});
            if let Some(owners) = owners {
                for (pid, name) in owners {
                    row["pid"] = json!(pid);
                    row["process"] = json!(name);
                    rows.push(row.clone());
                    if kill.is_some() {
                        guard(*pid, name)?;
                        eprintln!("{pid} {name} {}", s.addr);
                        targets.insert((*pid, name.clone(), s.inode));
                    }
                }
            } else {
                ensure!(kill.is_none(), "listener owner unavailable; try sudo");
                rows.push(row)
            }
        }
        let mut v = json!({"sockets":rows});
        if hidden {
            eprintln!("Use sudo to see hidden processes.");
        }
        if let Some((_, force, yes)) = kill {
            if !yes {
                eprint!("Terminate these processes? [y/N] ");
                io::stderr().flush()?;
                let mut answer = String::new();
                io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    v["terminated"] = json!(false);
                    return Ok(v);
                }
            }
            let mut handles = BTreeMap::new();
            for (pid, _, _) in &targets {
                if let std::collections::btree_map::Entry::Vacant(entry) = handles.entry(*pid) {
                    // The descriptor keeps the signal bound to this process if its PID is reused.
                    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, *pid, 0u32) };
                    if fd < 0 {
                        return Err(io::Error::last_os_error().into());
                    }
                    entry.insert(unsafe { OwnedFd::from_raw_fd(fd as i32) });
                }
            }
            let (current, _) = owners(&targets.iter().map(|t| t.2).collect())?;
            for (pid, name, inode) in targets {
                ensure!(
                    current
                        .get(&inode)
                        .is_some_and(|s| s.contains(&(pid, name.clone()))),
                    "listener changed"
                );
                guard(pid, &name)?;
            }
            for (pid, fd) in handles {
                let exe = fs::read_link(format!("/proc/{pid}/exe"))?;
                let name = exe.file_name().unwrap_or_default().to_string_lossy();
                guard(pid, name.trim_end_matches(" (deleted)"))?;
                let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
                let status = unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        fd.as_raw_fd(),
                        signal,
                        std::ptr::null::<libc::siginfo_t>(),
                        0u32,
                    )
                };
                if status < 0 {
                    return Err(io::Error::last_os_error().into());
                }
            }
            v["terminated"] = json!(true)
        }
        if !machine && kill.is_none() {
            let mut table = vec![(
                "Protocol".into(),
                "Address  State  Exposure  PID  Process".into(),
            )];
            for row in rows {
                table.push((
                    row["protocol"].as_str().unwrap().into(),
                    format!(
                        "{}  {}  {}  {}  {}",
                        row["address"].as_str().unwrap(),
                        row["state"].as_str().unwrap(),
                        row["exposure"].as_str().unwrap(),
                        row["pid"]
                            .as_u64()
                            .map(|x| x.to_string())
                            .unwrap_or_else(|| "-".into()),
                        row["process"].as_str().unwrap()
                    ),
                ))
            }
            chonk_lib::table(&table);
            return Ok(Value::Null);
        }
        Ok(v)
    }
    pub fn net() -> Result<Value> {
        let interfaces=chonk_lib::output("ip",&["-j","addr","show","up"]).ok().and_then(|s|serde_json::from_str::<Vec<Value>>(&s).ok()).map(|items|items.into_iter().map(|i|json!({"name":i["ifname"],"addresses":i["addr_info"].as_array().into_iter().flatten().map(|a|&a["local"]).collect::<Vec<_>>()})).collect::<Vec<_>>()).unwrap_or_else(||fs::read_dir("/sys/class/net").into_iter().flatten().flatten().filter(|e|fs::read_to_string(e.path().join("operstate")).is_ok_and(|s|matches!(s.trim(),"up"|"unknown"))).map(|e|json!({"name":e.file_name().to_string_lossy()})).collect());
        let dns = fs::read_to_string("/etc/resolv.conf")?
            .lines()
            .filter_map(|l| {
                let mut f = l.split_whitespace();
                (f.next() == Some("nameserver"))
                    .then(|| f.next())
                    .flatten()
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>();
        let tailscale = match Command::new("tailscale")
            .args(["status", "--json"])
            .output()
        {
            Ok(o) => serde_json::from_slice::<Value>(&o.stdout)
                .ok()
                .map(|v| v["BackendState"].clone())
                .unwrap_or(json!("unavailable")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => json!("not installed"),
            Err(_) => json!("unavailable"),
        };
        let start = Instant::now();
        let latency = TcpStream::connect_timeout(&"1.1.1.1:53".parse()?, Duration::from_secs(2))
            .ok()
            .map(|_| start.elapsed().as_millis());
        Ok(
            json!({"interfaces":interfaces,"gateway":gateway(&fs::read_to_string("/proc/net/route")?),"dns":dns,"tailscale":tailscale,"latency_ms":latency}),
        )
    }
}
fn local_name(name: &str) -> bool {
    let name = name.trim_end_matches('.').as_bytes();
    name.get(name.len().saturating_sub(6)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(b".local"))
}

fn resolution_error(name: &str, error: impl std::fmt::Display) -> anyhow::Error {
    if local_name(name) {
        anyhow::anyhow!("DNS resolution for {name}: {error}. Static musl does not use NSS/mDNS (/etc/nsswitch.conf). Use an IP address or enable systemd-resolved with mDNS and point /etc/resolv.conf at its 127.0.0.53 stub.")
    } else {
        anyhow::anyhow!("DNS resolution for {name}: {error}")
    }
}

fn resolve(name: &str, address: impl ToSocketAddrs) -> Result<Vec<std::net::SocketAddr>> {
    let addrs = address
        .to_socket_addrs()
        .map_err(|e| resolution_error(name, e))?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return Err(resolution_error(name, "no addresses"));
    }
    Ok(addrs)
}

fn run(cli: Cli) -> Result<(Value, i32)> {
    match cli.cmd {
        Cmd::Watch {
            host,
            timeout,
            interval,
        } => {
            ensure!(interval > 0, "interval must be positive");
            let start = Instant::now();
            let limit = Duration::from_secs(timeout);
            let step = Duration::from_millis(interval);
            let (tx, rx) = std::sync::mpsc::channel();
            let name = host
                .rsplit_once(':')
                .map_or(host.as_str(), |(name, _)| name)
                .to_owned();
            let lookup_name = name.clone();
            std::thread::spawn(move || {
                let _ = tx.send(resolve(&lookup_name, host));
            });
            if let Ok(addrs) = rx.recv_timeout(limit.saturating_sub(start.elapsed())) {
                let addrs = addrs?;
                while start.elapsed() < limit {
                    let tick = Instant::now();
                    for a in &addrs {
                        let budget = limit.saturating_sub(start.elapsed()).min(step);
                        if !budget.is_zero() && TcpStream::connect_timeout(a, budget).is_ok() {
                            return Ok((
                                json!({"open":true,"wait_ms":start.elapsed().as_millis()}),
                                0,
                            ));
                        }
                    }
                    std::thread::sleep(
                        step.saturating_sub(tick.elapsed())
                            .min(limit.saturating_sub(start.elapsed())),
                    );
                }
            } else if local_name(&name) {
                return Err(resolution_error(&name, "lookup timed out"));
            }
            Ok((
                json!({"open":false,"wait_ms":start.elapsed().as_millis()}),
                1,
            ))
        }
        #[cfg(target_os = "linux")]
        Cmd::Ports { all } => Ok((linux::inspect(all, None, cli.json)?, 0)),
        #[cfg(target_os = "linux")]
        Cmd::Kill { port, force, yes } => Ok((
            linux::inspect(false, Some((port, force, yes)), cli.json)?,
            0,
        )),
        #[cfg(target_os = "linux")]
        Cmd::Net { host } => {
            let addresses = host
                .as_deref()
                .map(|name| {
                    resolve(name, (name, 0)).map(|items| {
                        items
                            .into_iter()
                            .map(|addr| addr.ip())
                            .collect::<std::collections::BTreeSet<_>>()
                    })
                })
                .transpose()?;
            let mut net = linux::net()?;
            if let Some(addresses) = addresses {
                net["resolution"] = json!({"host":host,"addresses":addresses});
            }
            Ok((net, 0))
        }
        #[cfg(not(target_os = "linux"))]
        _ => anyhow::bail!("this command requires Linux"),
    }
}
fn main() {
    let cli = Cli::parse();
    let machine = cli.json;
    let result = run(cli);
    if matches!(&result, Ok((Value::Null, 0))) && !machine {
        return;
    }
    std::process::exit(chonk_lib::report(result, machine))
}
#[cfg(test)]
mod tests {
    use super::procnet::*;

    #[test]
    fn local_dns_hint() {
        for name in ["printer.local", "printer.LOCAL.", "ł.local"] {
            assert!(super::resolution_error(name, "failed")
                .to_string()
                .contains("127.0.0.53"));
        }
        assert!(!super::resolution_error("local.example", "failed")
            .to_string()
            .contains("NSS"));
    }
    #[test]
    fn fixtures() {
        for (proto, raw, expected) in [
            ("tcp", "0100007F", "127.0.0.1:22"),
            ("tcp6", "00000000000000000000000001000000", "[::1]:22"),
        ] {
            let s = format!(" 0: {raw}:0016 unused 0A 0 0 0 1000 0 42");
            let row = parse(&s, proto, false).pop().unwrap();
            assert_eq!(row.addr.to_string(), expected);
            assert_eq!(
                (row.inode, row.proto.as_str(), row.state.as_str()),
                (42, proto, "0A")
            );
            assert!(parse(&s.replace("0A", "01"), proto, false).is_empty())
        }
        assert_eq!(
            gateway("eth0 00000000 0101A8C0 0003").unwrap().to_string(),
            "192.168.1.1"
        )
    }
    #[test]
    fn protected() {
        for pid in [0, 1, std::process::id()] {
            assert!(guard(pid, "other").is_err())
        }
        assert!(guard(999999, "sshd").is_err());
        assert!(guard(999999, "sshd-session").is_err());
        assert!(<super::Cli as clap::Parser>::try_parse_from(["whisker", "kill", "0"]).is_err());
    }
}
