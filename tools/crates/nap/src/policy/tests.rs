use super::{dark, guard};
use crate::cli::{Cli, Dark, Guard, Limit};
use anyhow::Result;
use clap::Parser;
use std::{fs, path::Path};

fn put(root: &Path, path: &str, value: &str) -> Result<()> {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(file, value)?;
    Ok(())
}

#[test]
fn guard_prefers_ideapad_and_reports_temperature() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path();
    let mode = "sys/bus/platform/drivers/ideapad_acpi/VPC2004:00/conservation_mode";
    let threshold = "sys/class/power_supply/BAT0/charge_control_end_threshold";
    let config = "etc/tlp.d/95-chonkos-guard.conf";
    // A Windows-compatible fixture name still exercises the VPC* match.
    let mode = if cfg!(windows) {
        mode.replace(':', "_")
    } else {
        mode.to_owned()
    };
    put(root, &mode, "0")?;
    assert_eq!(guard(root, Guard::Status)?["conservation_mode"], 0);
    put(root, threshold, "100")?;
    put(root, "sys/class/thermal/thermal_zone0/temp", "32500")?;
    put(root, "sys/class/thermal/thermal_zone1/temp", "42375")?;
    let enabled = guard(
        root,
        Guard::Enable {
            limit: Limit::Eighty,
        },
    )?;
    assert_eq!(enabled["driver"], "ideapad_acpi");
    assert_eq!(guard(root, Guard::Status)?["conservation_mode"], 1);
    assert_eq!(enabled["enabled"], true);
    assert_eq!(enabled["max_temperature_c"], 42.375);
    assert_eq!(fs::read_to_string(root.join(&mode))?, "1");
    assert_eq!(fs::read_to_string(root.join(threshold))?, "100");
    assert_eq!(
        fs::read_to_string(root.join(config))?,
        "START_CHARGE_THRESH_BAT0=0\nSTOP_CHARGE_THRESH_BAT0=1\n"
    );
    assert_eq!(guard(root, Guard::Status)?["tlp_drop_in"], true);
    guard(root, Guard::Disable)?;
    assert_eq!(fs::read_to_string(root.join(mode))?, "0");
    assert!(!root.join(config).exists());
    Ok(())
}

#[test]
fn guard_thresholds_supported_and_unsupported() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path();
    let end = "sys/class/power_supply/BAT0/charge_control_end_threshold";
    let start = "sys/class/power_supply/BAT0/charge_control_start_threshold";
    let config = "etc/tlp.d/95-chonkos-guard.conf";
    fs::create_dir(root.join("sys"))?;
    let status = serde_json::to_string(&guard(root, Guard::Status)?)?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&status)?,
        serde_json::json!({
            "driver":"unsupported","enabled":null,"tlp_drop_in":false,"max_temperature_c":null
        })
    );
    assert_eq!(
        guard(
            root,
            Guard::Enable {
                limit: Limit::Sixty
            }
        )?["driver"],
        "unsupported"
    );
    assert!(!root.join(config).exists());
    put(root, end, "100")?;
    let enabled = guard(
        root,
        Guard::Enable {
            limit: Limit::Sixty,
        },
    )?;
    assert_eq!(enabled["driver"], "generic_sysfs");
    assert_eq!(enabled["start_threshold"], serde_json::Value::Null);
    assert_eq!(enabled["end_threshold"], 60);
    put(root, start, "55")?;
    fs::create_dir_all(root.join("sys/devices/platform/thinkpad_acpi"))?;
    let enabled = guard(
        root,
        Guard::Enable {
            limit: Limit::Eighty,
        },
    )?;
    assert_eq!(enabled["driver"], "thinkpad_acpi");
    assert_eq!(enabled["start_threshold"], 75);
    assert_eq!(enabled["end_threshold"], 80);
    assert_eq!(
        fs::read_to_string(root.join(config))?,
        "START_CHARGE_THRESH_BAT0=75\nSTOP_CHARGE_THRESH_BAT0=80\n"
    );
    let lowered = guard(
        root,
        Guard::Enable {
            limit: Limit::Sixty,
        },
    )?;
    assert_eq!(lowered["start_threshold"], 55);
    assert_eq!(lowered["end_threshold"], 60);
    assert_eq!(
        fs::read_to_string(root.join(config))?,
        "START_CHARGE_THRESH_BAT0=55\nSTOP_CHARGE_THRESH_BAT0=60\n"
    );
    assert_eq!(guard(root, Guard::Disable)?["end_threshold"], 100);
    assert!(!root.join(config).exists());
    for invalid in ["0", "70", "105"] {
        let error = Cli::try_parse_from(["nap", "guard", "enable", "--limit", invalid])
            .err()
            .expect("invalid limit accepted");
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
        assert_eq!(error.exit_code(), 2);
    }
    assert!(Cli::try_parse_from(["nap", "guard", "status", "--limit", "80"]).is_err());
    Ok(())
}

#[test]
fn dark_round_trip_preserves_brightness_and_applies_settings() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path();
    let power = "sys/class/backlight/intel_backlight/bl_power";
    let panel_level = "sys/class/backlight/intel_backlight/brightness";
    put(root, panel_level, "80")?;
    let brightness = "sys/class/backlight/fallback/brightness";
    let state = "var/lib/chonkos/backlight-prev";
    let wifi = "etc/NetworkManager/conf.d/90-chonkos-wifi.conf";
    put(root, power, "0")?;
    put(root, brightness, "123")?;
    put(root, "sys/module/kernel/parameters/consoleblank", "0")?;
    put(root, "sys/class/power_supply/BAT0/type", "Battery")?;
    put(root, "sys/class/power_supply/BAT0/power_now", "7200000")?;
    fs::create_dir_all(root.join("etc/NetworkManager/conf.d"))?;
    let on = dark(root, Dark::On)?;
    assert_eq!(on["enabled"], true);
    assert_eq!(on["consoleblank"], 60);
    assert_eq!(on["power_watts"]["before"], 7.2);
    assert_eq!(on["power_watts"]["delta"], 0.);
    assert_eq!(fs::read_to_string(root.join(power))?, "4");
    assert_eq!(fs::read_to_string(root.join(panel_level))?, "80");
    assert_eq!(fs::read_to_string(root.join(brightness))?, "0");
    assert_eq!(
        fs::read_to_string(root.join(wifi))?,
        "[connection]\nwifi.powersave = 2\n"
    );
    dark(root, Dark::On)?;
    assert_eq!(dark(root, Dark::Status)?["wifi_drop_in"], true);
    let off = dark(root, Dark::Off)?;
    assert_eq!(off["enabled"], false);
    assert_eq!(fs::read_to_string(root.join(power))?, "0");
    assert_eq!(fs::read_to_string(root.join(panel_level))?, "80");
    assert_eq!(fs::read_to_string(root.join(brightness))?, "123");
    assert!(!root.join(state).exists());
    assert!(!root.join(wifi).exists());
    Ok(())
}

#[test]
fn dark_on_without_optional_interfaces() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let result = dark(tmp.path(), Dark::On)?;
    assert!(result["enabled"].is_null());
    assert!(result["power_watts"]["delta"].is_null());
    assert_eq!(result["wifi_drop_in"], false);
    assert!(result["warnings"].to_string().contains("consoleblank=60"));
    assert!(!tmp.path().join("etc").exists());
    assert!(!tmp.path().join("sys").exists());
    Ok(())
}
