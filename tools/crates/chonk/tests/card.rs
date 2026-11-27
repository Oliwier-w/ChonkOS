use serde_json::Value;
use std::process::Command;
use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};

#[test]
fn card_json_has_version_and_utc_timestamp() {
    let output = Command::new(env!("CARGO_BIN_EXE_chonk"))
        .args(["card", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let card: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(card["schema_version"], 1);
    let stamp = card["timestamp"].as_str().unwrap();
    assert_eq!(
        OffsetDateTime::parse(stamp, &Rfc3339).unwrap().offset(),
        UtcOffset::UTC
    );
}

#[test]
fn card_supports_no_color() {
    let output = Command::new(env!("CARGO_BIN_EXE_chonk"))
        .arg("card")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("ChonkOS"));
    assert!(!text.contains("\x1b["));
}
#[test]
fn waybar_prints_one_json_line() {
    let temp = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_chonk"))
        .args(["mood", "--waybar"])
        .env("PATH", temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["text"], "CHONK");
    let state = value["alt"].as_str().unwrap();
    assert!(matches!(state, "hot" | "calm"));
    assert_eq!(
        value["class"],
        serde_json::json!(["chonky", format!("state-{state}")])
    );
    let tooltip = value["tooltip"].as_str().unwrap();
    assert!(tooltip.starts_with(&format!("State: {state}\nPSS: ")));
    for label in ["\nZRAM: ", "\nTemp: ", "\nPorts: "] {
        assert!(tooltip.contains(label));
    }
}
