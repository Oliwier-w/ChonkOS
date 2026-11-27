use std::{fs, process::Command};

#[test]
fn apply_validates_before_writing_and_preserves_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("chonkos");
    let generated = base.join("generated");
    let overrides = base.join("overrides");
    fs::create_dir_all(&overrides).unwrap();
    fs::create_dir(overrides.join("sway")).unwrap();
    let custom = overrides.join("sway/90-custom.conf");
    let custom_bytes = b"# Preserve these bytes\r\nset $custom 1\n";
    fs::write(&custom, custom_bytes).unwrap();
    let apply = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_chonk"))
            .args(["apply", "--json"])
            .args(args)
            .env("XDG_CONFIG_HOME", temp.path())
            .env("PATH", temp.path())
            .output()
            .unwrap()
    };
    assert!(apply(&["--dry-run"]).status.success());
    assert!(!generated.exists());
    assert!(apply(&[]).status.success());
    assert_eq!(generated.read_dir().unwrap().count(), 8);
    let theme = generated.join("waybar-colors.css");
    let original = fs::read_to_string(&theme).unwrap();
    assert!(original.contains("#0E141B"));
    fs::write(
        base.join("chonk.toml"),
        "schema_version=1\n[appearance]\nfont_mono=\"bad;directive\"",
    )
    .unwrap();
    assert!(!apply(&[]).status.success());
    assert_eq!(fs::read_to_string(&theme).unwrap(), original);
    fs::write(base.join("chonk.toml"),"schema_version=1\n[appearance]\npalette=\"seafoam-light\"\nchonky_waybar=false\nnvidia_wayland_experimental=true").unwrap();
    assert!(apply(&[]).status.success());
    assert!(fs::read_to_string(theme).unwrap().contains("opacity: 0"));
    assert_eq!(
        fs::read_to_string(generated.join("session-env")).unwrap(),
        "CHONKOS_NVIDIA_EXPERIMENTAL=1\n"
    );
    for (palette, colors) in [
        (
            "arctic-slate",
            [
                "0E141B", "16202A", "263646", "E6EEF5", "8CA0B3", "3B82A6", "38BDF8", "F59E0B",
                "EF4444", "22C55E",
            ],
        ),
        (
            "polar-night",
            [
                "090D12", "121923", "1E293B", "E2E8F0", "94A3B8", "22D3EE", "38BDF8", "F59E0B",
                "EF4444", "22C55E",
            ],
        ),
        (
            "seafoam-light",
            [
                "F7F9FB", "EBF1F5", "CBD5E1", "0F172A", "475569", "0369A1", "0E7490", "D97706",
                "DC2626", "16A34A",
            ],
        ),
    ] {
        fs::write(
            base.join("chonk.toml"),
            format!("schema_version=1\n[appearance]\npalette=\"{palette}\""),
        )
        .unwrap();
        assert!(apply(&[]).status.success());
        let [bg, surface, border, fg, muted, accent, cyan, warn, alert, ok] = colors;
        for (name, expected) in [
            (
                "sway-colors.conf",
                vec![
                    format!("client.focused #{accent} #{surface} #{fg} #{cyan} #{accent}"),
                    format!("client.unfocused #{border} #{bg} #{muted} #{border} #{border}"),
                ],
            ),
            (
                "waybar-colors.css",
                vec![
                    format!("background: #{bg}; color: #{fg}"),
                    format!("opacity: 1; color: #{ok}"),
                    format!("state-hot {{ color: #{alert}"),
                    format!("state-alert {{ color: #{warn}"),
                    format!("state-seal {{ color: #{cyan}"),
                ],
            ),
            (
                "foot-theme.ini",
                vec![format!(
                    "background={bg}\nforeground={fg}\nregular4={accent}\n"
                )],
            ),
            (
                "fuzzel-theme.ini",
                vec![format!(
                    "background={bg}ff\ntext={fg}ff\nselection={surface}ff\nborder={border}ff\n"
                )],
            ),
            (
                "mako-theme.conf",
                vec![format!(
                    "background-color=#{surface}\ntext-color=#{fg}\nborder-color=#{accent}"
                )],
            ),
            (
                "tmux-theme.conf",
                vec![
                    format!("bg=#{surface},fg=#{fg}"),
                    format!("pane-active-border-style 'fg=#{accent}'"),
                ],
            ),
            (
                "shell-palette.sh",
                vec![format!("bg:#{bg},fg:#{fg},hl:#{cyan},pointer:#{accent}")],
            ),
        ] {
            let actual = fs::read_to_string(generated.join(name)).unwrap();
            for fragment in expected {
                assert!(
                    actual.contains(&fragment),
                    "{palette} {name} missing {fragment}"
                );
            }
        }
        assert!(fs::read(generated.join("session-env")).unwrap().is_empty());
        assert_eq!(generated.read_dir().unwrap().count(), 8);
        assert_eq!(fs::read(&custom).unwrap(), custom_bytes);
    }
}
