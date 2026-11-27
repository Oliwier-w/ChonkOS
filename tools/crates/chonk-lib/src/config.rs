use std::collections::BTreeSet;

#[derive(Debug)]
pub struct Config {
    pub mode: String,
    pub profile: String,
    pub battery_guard: String,
    pub lid_homelab: String,
    pub dark_console_blank_sec: u32,
    pub dive_shutdown_percent: u8,
    pub palette: String,
    pub chonky_waybar: bool,
    pub font_mono: String,
    pub font_size: u8,
    pub nvidia_wayland_experimental: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: "desktop".into(),
            profile: "dev".into(),
            battery_guard: "auto".into(),
            lid_homelab: "ignore".into(),
            dark_console_blank_sec: 60,
            dive_shutdown_percent: 20,
            palette: "arctic-slate".into(),
            chonky_waybar: true,
            font_mono: "JetBrains Mono".into(),
            font_size: 11,
            nvidia_wayland_experimental: false,
        }
    }
}

fn string(value: &str) -> Option<&str> {
    let text = value.strip_prefix('"')?.strip_suffix('"')?;
    (!text.contains(['"', '\\']) && !text.chars().any(char::is_control)).then_some(text)
}

fn choice(value: &str, allowed: &[&str]) -> Option<String> {
    string(value)
        .filter(|s| allowed.contains(s))
        .map(str::to_owned)
}

impl Config {
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut config = Self::default();
        let mut section = "";
        let mut seen = BTreeSet::new();
        for (index, raw) in source.lines().enumerate() {
            let error = || format!("Invalid configuration at line {}", index + 1);
            let mut quoted = false;
            let end = raw
                .char_indices()
                .find_map(|(i, ch)| {
                    if ch == '"' {
                        quoted = !quoted;
                    }
                    (ch == '#' && !quoted).then_some(i)
                })
                .unwrap_or(raw.len());
            let line = raw[..end].trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                section = line
                    .strip_prefix('[')
                    .and_then(|s| s.strip_suffix(']'))
                    .ok_or_else(error)?;
                if !matches!(section, "system" | "power" | "appearance")
                    || !seen.insert((section, ""))
                {
                    return Err(error());
                }
                continue;
            }
            let (key, value) = line.split_once('=').ok_or_else(error)?;
            let key = key.trim();
            let value = value.trim();
            if !seen.insert((section, key)) {
                return Err(error());
            }
            match (section, key) {
                ("", "schema_version") if value == "1" => (),
                ("system", "mode") => {
                    config.mode = choice(value, &["desktop", "homelab"]).ok_or_else(error)?
                }
                ("system", "profile") => {
                    config.profile =
                        choice(value, &["dev", "battery", "homelab"]).ok_or_else(error)?
                }
                ("power", "battery_guard") => {
                    config.battery_guard =
                        choice(value, &["auto", "off", "60", "80"]).ok_or_else(error)?
                }
                ("power", "lid_homelab") => {
                    config.lid_homelab = choice(value, &["ignore", "sleep"]).ok_or_else(error)?
                }
                ("power", "dark_console_blank_sec") => {
                    config.dark_console_blank_sec = value.parse().map_err(|_| error())?
                }
                ("power", "dive_shutdown_percent") => {
                    config.dive_shutdown_percent = value
                        .parse()
                        .ok()
                        .filter(|n| (10..=50).contains(n))
                        .ok_or_else(error)?
                }
                ("appearance", "palette") => {
                    config.palette =
                        choice(value, &["arctic-slate", "polar-night", "seafoam-light"])
                            .ok_or_else(error)?
                }
                ("appearance", "chonky_waybar") => {
                    config.chonky_waybar = value.parse().map_err(|_| error())?
                }
                ("appearance", "nvidia_wayland_experimental") => {
                    config.nvidia_wayland_experimental = value.parse().map_err(|_| error())?
                }
                ("appearance", "font_size") => {
                    config.font_size = value
                        .parse()
                        .ok()
                        .filter(|n| (8..=24).contains(n))
                        .ok_or_else(error)?
                }
                ("appearance", "font_mono") => {
                    config.font_mono = string(value)
                        .filter(|s| {
                            !s.trim().is_empty()
                                && s.bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b" ._-".contains(&b))
                        })
                        .ok_or_else(error)?
                        .into();
                }
                _ => return Err(error()),
            }
        }
        if !seen.contains(&("", "schema_version")) {
            return Err("Missing schema_version".into());
        }
        Ok(config)
    }

    pub fn colors(&self) -> [&'static str; 10] {
        match self.palette.as_str() {
            "polar-night" => [
                "#090D12", "#121923", "#1E293B", "#E2E8F0", "#94A3B8", "#22D3EE", "#38BDF8",
                "#F59E0B", "#EF4444", "#22C55E",
            ],
            "seafoam-light" => [
                "#F7F9FB", "#EBF1F5", "#CBD5E1", "#0F172A", "#475569", "#0369A1", "#0E7490",
                "#D97706", "#DC2626", "#16A34A",
            ],
            _ => [
                "#0E141B", "#16202A", "#263646", "#E6EEF5", "#8CA0B3", "#3B82A6", "#38BDF8",
                "#F59E0B", "#EF4444", "#22C55E",
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_and_partial_configs() {
        let c = Config::parse(
            r#"schema_version=1
[system]
mode="homelab"
profile="battery"
[power]
battery_guard="80"
lid_homelab="sleep"
dark_console_blank_sec=120
dive_shutdown_percent=30
[appearance]
palette="seafoam-light"
chonky_waybar=false
font_mono="Test Mono"
font_size=16
nvidia_wayland_experimental=true"#,
        )
        .unwrap();
        assert_eq!(
            (
                c.mode.as_str(),
                c.profile.as_str(),
                c.battery_guard.as_str(),
                c.lid_homelab.as_str()
            ),
            ("homelab", "battery", "80", "sleep")
        );
        assert_eq!(
            (c.dark_console_blank_sec, c.dive_shutdown_percent),
            (120, 30)
        );
        assert_eq!(
            (c.palette.as_str(), c.font_mono.as_str(), c.font_size),
            ("seafoam-light", "Test Mono", 16)
        );
        assert!(!c.chonky_waybar && c.nvidia_wayland_experimental);
        let partial = Config::parse("schema_version=1\n[appearance]\nfont_size=13").unwrap();
        assert_eq!(partial.font_size, 13);
        assert_eq!(
            (
                partial.mode.as_str(),
                partial.profile.as_str(),
                partial.battery_guard.as_str(),
                partial.lid_homelab.as_str()
            ),
            ("desktop", "dev", "auto", "ignore")
        );
        assert_eq!(
            (
                partial.dark_console_blank_sec,
                partial.dive_shutdown_percent
            ),
            (60, 20)
        );
        assert_eq!(
            (partial.palette.as_str(), partial.font_mono.as_str()),
            ("arctic-slate", "JetBrains Mono")
        );
        assert!(partial.chonky_waybar && !partial.nvidia_wayland_experimental);
    }
    #[test]
    fn validates_schema_and_fields() {
        for text in [
            "",
            "schema_version=99",
            "schema_version=1\nschema_version=1",
            "schema_version=1\n[other]",
        ] {
            assert!(Config::parse(text).is_err());
        }
        for field in [
            "font_size=7",
            "font_size=25",
            "chonky_waybar=1",
            "palette=\"blue\"",
            "font_mono=\"a;b\"",
            "font_mono=\"$x\"",
            "font_mono=\"a\\b\"",
            "font_mono=\"a'b\"",
            "font_mono=\"Mono\nexec /bin/sh\"",
        ] {
            assert!(Config::parse(&format!("schema_version=1\n[appearance]\n{field}")).is_err());
        }
        let c = Config::parse("schema_version=1\n[system]\nmode=\"homelab\"\n[power]\ndive_shutdown_percent=30\n[appearance]\nfont_size=24 # large\npalette=\"polar-night\"").unwrap();
        assert_eq!(
            (
                c.mode.as_str(),
                c.dive_shutdown_percent,
                c.font_size,
                c.colors()[0]
            ),
            ("homelab", 30, 24, "#090D12")
        );
    }
}
