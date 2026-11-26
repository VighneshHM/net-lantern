//! The bundle must be a per-user NSIS installer whose uninstaller cleans up
//! after Net Lantern without touching anything on an update install.

use std::{fs, path::Path};

use serde_json::Value;

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn config() -> Value {
    let text = fs::read_to_string(manifest_dir().join("tauri.conf.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn bundle_config_is_a_per_user_nsis_installer() {
    let config = config();
    let bundle = &config["bundle"];
    assert_eq!(bundle["active"], Value::Bool(true));
    assert_eq!(bundle["targets"], serde_json::json!(["nsis"]));

    let windows = &bundle["windows"];
    let nsis = &windows["nsis"];
    assert_eq!(nsis["installMode"], "currentUser");
    assert_eq!(nsis["installerHooks"], "windows/hooks.nsh");
    assert!(
        manifest_dir().join("windows/hooks.nsh").is_file(),
        "the installer hooks file is missing"
    );
    assert_eq!(windows["webviewInstallMode"]["type"], "downloadBootstrapper");

    assert_eq!(config["identifier"], "dev.netlantern.app");
    assert!(
        config.get("version").is_none(),
        "the version comes from Cargo.toml, not tauri.conf.json"
    );
}

#[test]
fn the_hooks_skip_updates_and_name_the_right_value() {
    let hooks = fs::read_to_string(manifest_dir().join("windows/hooks.nsh")).unwrap();
    assert!(hooks.contains("\"Net Lantern\""), "the Run value name");
    assert!(hooks.contains(r"$LOCALAPPDATA\NetLantern"), "the data folder");
    assert!(hooks.contains("${If} $UpdateMode <> 1"), "the update-install guard");
    assert!(
        hooks.contains(r"Explorer\StartupApproved\Run"),
        "Task Manager's switch for the Run value"
    );
    assert!(hooks.contains("$DeleteAppDataCheckboxState = 1"), "the checkbox");
    assert!(hooks.contains(
        "If you turned on phone access, Windows Firewall may still have rules that allow \
         Net Lantern. Removing them needs an administrator: open Windows Security, then \
         Firewall & network protection, Advanced settings, Inbound Rules, and delete the \
         rules for Net Lantern."
    ));
}

#[test]
fn the_delete_data_checkbox_says_what_it_deletes() {
    let config = config();
    let file = config["bundle"]["windows"]["nsis"]["customLanguageFiles"]["English"]
        .as_str()
        .expect("an English language file");
    let english = fs::read_to_string(manifest_dir().join(file)).unwrap();
    assert!(english.contains(
        "LangString deleteAppData ${LANG_ENGLISH} \
         \"Also delete Net Lantern's data (devices, history, settings and logs)\""
    ));
}
