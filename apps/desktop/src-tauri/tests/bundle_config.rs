//! The bundle must be a per-user NSIS installer whose uninstaller cleans up
//! after Net Lantern without touching anything on an update install.

use std::{fs, path::Path};

use netlantern_platform::startup::{APPROVED_KEY, RUN_KEY, VALUE_NAME};
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
    assert_eq!(nsis["displayLanguageSelector"], Value::Bool(false));
    assert_eq!(
        windows["webviewInstallMode"]["type"],
        "downloadBootstrapper"
    );
    assert_eq!(bundle["publisher"], "Net Lantern");
    assert_eq!(
        bundle["shortDescription"],
        "See what's on your Wi-Fi and why the internet broke."
    );

    assert_eq!(config["identifier"], "dev.netlantern.app");
    assert!(
        config.get("version").is_none(),
        "the version comes from Cargo.toml, not tauri.conf.json"
    );
}

const UPGRADE_FILE: &str = r"$TEMP\netlantern-upgrade-run.txt";

fn hooks() -> String {
    fs::read_to_string(manifest_dir().join("windows/hooks.nsh")).unwrap()
}

/// The body of `!macro <name>`, up to its `!macroend`.
fn macro_body<'a>(hooks: &'a str, name: &str) -> &'a str {
    // Line ends may be CRLF in a Windows checkout.
    let open = hooks
        .lines()
        .find(|l| l.trim_end() == format!("!macro {name}"))
        .unwrap_or_else(|| panic!("no macro {name}"));
    let start = hooks.find(open).unwrap() + open.len();
    let end = start + hooks[start..].find("!macroend").expect("!macroend");
    &hooks[start..end]
}

fn offset(body: &str, needle: &str) -> usize {
    body.find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?}"))
}

#[test]
fn the_hooks_skip_updates_and_name_the_right_value() {
    let hooks = hooks();

    // One flag decides whether the installer is driving this uninstall: an
    // updater run (/UPDATE) or the installer running uninstall.exe in place.
    let pre = macro_body(&hooks, "NSIS_HOOK_PREUNINSTALL");
    let decide = offset(pre, "StrCpy $NetLanternUpgrade 0");
    assert!(decide < offset(pre, "${If} $UpdateMode = 1"));
    assert!(decide < offset(pre, r#"${OrIf} $EXEFILE == "uninstall.exe""#));
    assert!(offset(pre, "StrCpy $NetLanternUpgrade 1") > decide);

    // Everything the real uninstall does sits inside that guard.
    let post = macro_body(&hooks, "NSIS_HOOK_POSTUNINSTALL");
    let guard = offset(post, "${If} $NetLanternUpgrade <> 1");
    let close = post.rfind("${EndIf}").expect("closing ${EndIf}");
    assert!(
        post[close + "${EndIf}".len()..]
            .lines()
            .all(|l| l.trim().is_empty() || l.trim().starts_with(';')),
        "nothing after the guard's ${{EndIf}}"
    );
    let run = format!(r#"DeleteRegValue HKCU "{RUN_KEY}" "{VALUE_NAME}""#);
    let approved = format!(r#"DeleteRegValue HKCU "{APPROVED_KEY}" "{VALUE_NAME}""#);
    let checkbox = "${If} $DeleteAppDataCheckboxState = 1";
    let rmdir = r#"RMDir /r "$LOCALAPPDATA\NetLantern""#;
    let message = "MessageBox MB_OK|MB_ICONINFORMATION \"If you turned on phone access, \
         Windows Firewall may still have rules that allow Net Lantern. Removing them needs \
         an administrator: open Windows Security, then Firewall & network protection, \
         Advanced settings, Inbound Rules, and delete the rules for Net Lantern.\"";
    let stale = format!(r#"Delete "{UPGRADE_FILE}""#);
    let real = &post[guard..close];
    for needle in [run.as_str(), &approved, checkbox, rmdir, message, &stale] {
        offset(real, needle);
    }
    assert!(offset(real, checkbox) < offset(real, rmdir));
}

#[test]
fn an_upgrade_keeps_start_at_login() {
    let hooks = hooks();
    // PREUNINSTALL runs before the template asks to close a running app, and
    // that prompt can still cancel the uninstall, so it only reads the value.
    let pre = macro_body(&hooks, "NSIS_HOOK_PREUNINSTALL");
    let read = offset(pre, "${If} $NetLanternUpgrade = 1");
    assert!(
        offset(
            pre,
            &format!(r#"ReadRegStr $NetLanternRunCmd HKCU "{RUN_KEY}" "{VALUE_NAME}""#)
        ) > read
    );
    assert!(
        !pre.contains("FileOpen") && !pre.contains(UPGRADE_FILE),
        "PREUNINSTALL never writes the upgrade file"
    );

    // POSTUNINSTALL runs only once the uninstall section has finished: its
    // upgrade branch saves the value, or clears a stale file if there is none.
    let post_un = macro_body(&hooks, "NSIS_HOOK_POSTUNINSTALL");
    let upgrade = offset(post_un, "${If} $NetLanternUpgrade = 1");
    let real = offset(post_un, "${If} $NetLanternUpgrade <> 1");
    assert!(
        upgrade < real,
        "the upgrade branch comes before the real uninstall"
    );
    let branch = &post_un[upgrade..real];
    let has_value = offset(branch, r#"${If} $NetLanternRunCmd != """#);
    let open = offset(branch, &format!(r#"FileOpen $1 "{UPGRADE_FILE}" w"#));
    let write = offset(branch, "FileWriteUTF16LE $1 $NetLanternRunCmd");
    let otherwise = offset(branch, "${Else}");
    let clear = offset(branch, &format!(r#"Delete "{UPGRADE_FILE}""#));
    assert!(has_value < open && open < write && write < otherwise && otherwise < clear);

    let post = macro_body(&hooks, "NSIS_HOOK_POSTINSTALL");
    let restore = offset(
        post,
        &format!(r#"${{If}} ${{FileExists}} "{UPGRADE_FILE}""#),
    );
    let write = offset(
        post,
        &format!(r#"WriteRegStr HKCU "{RUN_KEY}" "{VALUE_NAME}" $0"#),
    );
    let delete = offset(post, &format!(r#"Delete "{UPGRADE_FILE}""#));
    assert!(restore < write && write < delete);
    assert!(
        !hooks.contains(&format!(r#"WriteRegStr HKCU "{APPROVED_KEY}""#)),
        "Task Manager's switch is never written"
    );
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
         \"Also delete Net Lantern's data (devices, settings, logs)\""
    ));
}
