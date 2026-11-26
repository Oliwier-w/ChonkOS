use super::{entries, remove_if_present, system_command, write};
use crate::cli::Guard;
use anyhow::Result;
use chonk_lib::{metrics::read, write_public_config};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const DROP_IN: &str = "etc/tlp.d/95-chonkos-guard.conf";

enum Hardware {
    Conservation {
        control: PathBuf,
        battery: String,
    },
    Threshold {
        battery: String,
        start: PathBuf,
        end: PathBuf,
        thinkpad: bool,
    },
}

fn battery_name(root: &Path) -> Option<String> {
    entries(&root.join("sys/class/power_supply")).find_map(|path| {
        let name = path.file_name()?.to_str()?;
        let number = name.strip_prefix("BAT")?;
        (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then(|| name.to_owned())
    })
}

fn detect(root: &Path) -> Option<Hardware> {
    let battery = battery_name(root).unwrap_or_else(|| "BAT0".into());
    for path in entries(&root.join("sys/bus/platform/drivers/ideapad_acpi")) {
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("VPC"))
        {
            let control = path.join("conservation_mode");
            if control.is_file() {
                return Some(Hardware::Conservation { control, battery });
            }
        }
    }
    let thinkpad = root.join("sys/devices/platform/thinkpad_acpi").is_dir()
        || read(root.join("sys/class/dmi/id/product_family"))
            .is_some_and(|family| family.starts_with("ThinkPad"));
    entries(&root.join("sys/class/power_supply")).find_map(|path| {
        let battery = path.file_name()?.to_str()?;
        let number = battery.strip_prefix("BAT")?;
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let end = path.join("charge_control_end_threshold");
        end.is_file().then(|| Hardware::Threshold {
            battery: battery.into(),
            start: path.join("charge_control_start_threshold"),
            end,
            thinkpad,
        })
    })
}

fn number(path: &Path) -> Option<u8> {
    read(path)?.parse().ok()
}

impl Hardware {
    fn state(&self) -> Value {
        match self {
            Self::Conservation { control, .. } => {
                let mode = number(control);
                json!({"driver":"ideapad_acpi","enabled":mode.map(|n|n == 1),"conservation_mode":mode})
            }
            Self::Threshold {
                start,
                end,
                thinkpad,
                ..
            } => {
                let end = number(end);
                json!({"driver": if *thinkpad { "thinkpad_acpi" } else { "generic_sysfs" },
                    "enabled":end.map(|n| n > 0 && n < 100),
                    "start_threshold":number(start),"end_threshold":end})
            }
        }
    }

    fn enable(&self, limit: u8, drop_in: &Path) -> Result<()> {
        let (battery, start, end) = match self {
            Self::Conservation { control, battery } => {
                write(control, "1")?;
                (battery.as_str(), 0, 1)
            }
            Self::Threshold {
                battery,
                start,
                end,
                ..
            } => {
                let lower = limit - 5;
                let (lower_text, limit_text) = (lower.to_string(), limit.to_string());
                if start.is_file() && number(end).is_some_and(|old| old > lower) {
                    write(start, &lower_text)?;
                    write(end, &limit_text)?;
                } else {
                    write(end, &limit_text)?;
                    if start.is_file() {
                        write(start, &lower_text)?;
                    }
                }
                (battery.as_str(), lower, limit)
            }
        };
        write_public_config(
            drop_in,
            &format!("START_CHARGE_THRESH_{battery}={start}\nSTOP_CHARGE_THRESH_{battery}={end}\n"),
        )
    }

    fn disable(&self) -> Result<()> {
        match self {
            Self::Conservation { control, .. } => write(control, "0"),
            Self::Threshold { start, end, .. } => {
                write(end, "100")?;
                if start.is_file() {
                    write(start, "0")?;
                }
                Ok(())
            }
        }
    }
}

pub(in crate::policy) fn run(root: &Path, mode: Guard) -> Result<Value> {
    let drop_in = root.join(DROP_IN);
    let hardware = detect(root);
    let mut warnings = Vec::new();
    match mode {
        Guard::Enable { limit } => {
            if let Some(hw) = &hardware {
                hw.enable(limit as u8, &drop_in)?;
                if let Err(e) = system_command(root, "tlp", &["setcharge"]) {
                    warnings.push(format!("tlp setcharge: {e:#}"));
                }
            }
        }
        Guard::Disable => {
            if let Some(hw) = &hardware {
                hw.disable()?;
            }
            remove_if_present(&drop_in)?;
        }
        Guard::Status => {}
    }
    let mut result = hardware.as_ref().map_or_else(
        || json!({"driver":"unsupported","enabled":null}),
        Hardware::state,
    );
    result["tlp_drop_in"] = json!(drop_in.is_file());
    result["max_temperature_c"] = json!(entries(&root.join("sys/class/thermal"))
        .filter(|p| p
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("thermal_zone")))
        .filter_map(|p| read(p.join("temp"))?.parse::<i64>().ok())
        .max()
        .map(|t| t as f64 / 1000.));
    if !warnings.is_empty() {
        result["warnings"] = json!(warnings);
    }
    Ok(result)
}
