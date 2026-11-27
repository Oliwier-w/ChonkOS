use anyhow::{ensure, Context, Result};
use chonk_lib::{config::Config, metrics::read, output, write_public_config};
use serde_json::{json, Value};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

fn base() -> Result<PathBuf> {
    Ok(env::var_os("XDG_CONFIG_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .context("Set HOME or XDG_CONFIG_HOME")?
        .join("chonkos"))
}

fn load(path: Option<&Path>) -> Result<Config> {
    let default = base()?.join("chonk.toml");
    match fs::read_to_string(path.unwrap_or(&default)) {
        Ok(s) => Config::parse(&s).map_err(anyhow::Error::msg),
        Err(e) if path.is_none() && e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e.into()),
    }
}

fn themes(c: &Config) -> [(&'static str, String); 8] {
    let [bg, surface, border, fg, muted, accent, cyan, warn, alert, ok] = c.colors();
    let font = &c.font_mono;
    let size = c.font_size;
    let opacity = if c.chonky_waybar { 1 } else { 0 };
    [
        ("sway-colors.conf",format!("client.focused {accent} {surface} {fg} {cyan} {accent}\nclient.unfocused {border} {bg} {muted} {border} {border}\nfont pango:{font} {size}\n")),
        ("waybar-colors.css",format!("* {{ font-family: \"{font}\"; font-size: {size}pt; }}\nwindow#waybar {{ background: {bg}; color: {fg}; }}\n#custom-chonky {{ opacity: {opacity}; color: {ok}; }}\n#custom-chonky.state-hot {{ color: {alert}; }}\n#custom-chonky.state-alert {{ color: {warn}; }}\n#custom-chonky.state-seal {{ color: {cyan}; }}\n")),
        ("foot-theme.ini",format!("[main]\nfont={font}:size={size}\n[colors]\nbackground={}\nforeground={}\nregular4={}\n",&bg[1..],&fg[1..],&accent[1..])),
        ("fuzzel-theme.ini",format!("[main]\nfont={font}:size={size}\n[colors]\nbackground={}ff\ntext={}ff\nselection={}ff\nborder={}ff\n",&bg[1..],&fg[1..],&surface[1..],&border[1..])),
        ("mako-theme.conf",format!("font={font} {size}\nbackground-color={surface}\ntext-color={fg}\nborder-color={accent}\n")),
        ("tmux-theme.conf",format!("set -g status-style 'bg={surface},fg={fg}'\nset -g pane-active-border-style 'fg={accent}'\n")),
        ("shell-palette.sh",format!("export FZF_DEFAULT_OPTS='--color=bg:{bg},fg:{fg},hl:{cyan},pointer:{accent}'\n")),
        ("session-env",if c.nvidia_wayland_experimental {"CHONKOS_NVIDIA_EXPERIMENTAL=1\n".into()} else {String::new()}),
    ]
}

pub fn apply(path: Option<&Path>, dry: bool) -> Result<Value> {
    let files = themes(&load(path)?);
    let base = base()?;
    let dir = base.join("generated");
    for p in [&base, &dir] {
        match fs::symlink_metadata(p) {
            Ok(m) => ensure!(m.is_dir(), "Expected a directory at {}", p.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    let mut changed = Vec::new();
    for (name, contents) in &files {
        let path = dir.join(name);
        if path.is_symlink() || fs::read(&path).ok().as_deref() != Some(contents.as_bytes()) {
            changed.push(path);
        }
    }
    let mut warnings = Vec::new();
    if !dry {
        if chonk_lib::root() {
            let _ = output("seal", &["mark", "pre-apply"]);
        }
        for (name, data) in files {
            let path = dir.join(name);
            if changed.contains(&path) {
                write_public_config(&path, &data)?;
            }
        }
        reload(&mut warnings);
    }
    Ok(json!({"dry_run":dry,"files":changed,"warnings":warnings}))
}

fn reload(warnings: &mut Vec<String>) {
    let Some(uid) = read("/proc/self/status").and_then(|s| {
        s.lines().find_map(|line| {
            line.strip_prefix("Uid:")?
                .split_whitespace()
                .nth(1)?
                .parse::<u32>()
                .ok()
        })
    }) else {
        return;
    };
    let uid = uid.to_string();
    for (process, program, args) in [
        ("sway", "swaymsg", &["reload"][..]),
        ("mako", "makoctl", &["reload"]),
        (
            "waybar",
            "pkill",
            &["-u", uid.trim(), "-SIGUSR2", "-x", "waybar"],
        ),
        ("tmux", "tmux", &["source-file", "/etc/tmux.conf"]),
    ] {
        let active = if process == "tmux" {
            output("tmux", &["list-sessions"]).is_ok()
        } else {
            output("pgrep", &["-u", uid.trim(), "-x", process]).is_ok()
        };
        if active {
            if let Err(e) = output(program, args) {
                warnings.push(e.to_string());
            }
        }
    }
}

pub fn mode(name: &str, dark: bool, dry: bool) -> Result<Value> {
    let state = Path::new("/var/lib/chonkos/mode");
    let dash = "chonkos-tty-dash@tty1.service";
    if name == "status" {
        return Ok(
            json!({"mode":read(state),"default_target":output("systemctl",&["get-default"]).ok().map(|s|s.trim().to_owned()),"greetd_active":chonk_lib::unit_active("greetd.service"),"dashboard_active":chonk_lib::unit_active(dash)}),
        );
    }
    ensure!(dry || chonk_lib::root(), "Run with sudo");
    let c = load(None)?;
    let threshold = c.dive_shutdown_percent.to_string();
    let mut steps = Vec::new();
    if name == "homelab" {
        steps.push(vec!["systemctl", "set-default", "multi-user.target"]);
        steps.push(vec!["nap", "lid", "ignore"]);
        let guard = output("nap", &["guard", "status", "--json"])
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok());
        if guard
            .as_ref()
            .and_then(|v| v["driver"].as_str())
            .is_some_and(|d| d != "unsupported")
        {
            steps.push(vec!["nap", "guard", "enable"]);
        }
        steps.push(vec!["nap", "dive", "enable", "--threshold", &threshold]);
        if dark {
            steps.push(vec!["nap", "dark", "on"]);
        }
        steps.push(vec!["systemctl", "disable", "--now", "greetd.service"]);
        steps.push(vec!["systemctl", "enable", "--now", dash]);
        if !dry {
            write_public_config(state, "homelab\n")?;
        }
    } else {
        ensure!(
            ["/usr/sbin/greetd", "/usr/bin/greetd"]
                .iter()
                .any(|p| Path::new(p).is_file()),
            "Install greetd with sudo apt install chonkos-desktop"
        );
        steps.extend([
            vec!["systemctl", "disable", "--now", dash],
            vec!["nap", "dark", "off"],
            vec!["nap", "lid", "sleep"],
            vec!["systemctl", "set-default", "graphical.target"],
            vec!["systemctl", "enable", "--now", "greetd.service"],
        ]);
    }
    if !dry {
        for step in &steps {
            let result = output(step[0], &step[1..]);
            if step[1] != "disable" {
                result?;
            }
        }
        if name == "desktop" {
            write_public_config(state, "desktop\n")?;
        }
    }
    Ok(json!({"mode":name,"dry_run":dry,"steps":steps}))
}

pub fn doctor() -> (String, String, bool) {
    let config = load(None);
    let experimental = config.as_ref().is_ok_and(|c| c.nvidia_wayland_experimental);
    let major = read("/proc/driver/nvidia/version").and_then(|s| {
        s.split_whitespace()
            .filter_map(|t| t.split_once('.').and_then(|(n, _)| n.parse::<u32>().ok()))
            .next()
    });
    let warning =
        major.is_some_and(|n| n <= 550) && Path::new("/usr/bin/sway").is_file() && !experimental;
    let valid = config.is_ok();
    (
        config.map_or_else(|e| e.to_string(), |_| "ok".into()),
        if warning {
            "WARN: sudo apt install chonkos-desktop-x11"
        } else {
            "ok"
        }
        .into(),
        valid,
    )
}
