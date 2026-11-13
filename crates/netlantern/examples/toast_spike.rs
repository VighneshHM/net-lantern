//! Spike: can the unpackaged exe show a Windows toast, and under which app id?

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Mode {
    PowerShell,
    AppId,
    Register,
    Unregister,
}

fn parse_mode(arg: Option<&str>) -> Option<Mode> {
    match arg? {
        "--powershell" => Some(Mode::PowerShell),
        "--app-id" => Some(Mode::AppId),
        "--register" => Some(Mode::Register),
        "--unregister" => Some(Mode::Unregister),
        _ => None,
    }
}

#[cfg(windows)]
const APP_ID: &str = "NetLantern.NetLantern";

#[cfg(windows)]
fn key_path() -> Vec<u16> {
    format!(r"Software\Classes\AppUserModelId\{APP_ID}{}", "\0")
        .encode_utf16()
        .collect()
}

#[cfg(windows)]
fn show(app_id: &str) -> Result<(), String> {
    use tauri_winrt_notification::Toast;
    Toast::new(app_id)
        .title("Net Lantern test")
        .text1("If you can read this, toasts work.")
        .show()
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn register() -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    let path = key_path();
    let name: Vec<u16> = "DisplayName\0".encode_utf16().collect();
    let value: Vec<u16> = "Net Lantern\0".encode_utf16().collect();
    let bytes: Vec<u8> = value.iter().flat_map(|u| u.to_le_bytes()).collect();
    let mut key = HKEY::default();
    unsafe {
        let r = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        );
        if r != ERROR_SUCCESS {
            return Err(format!("RegCreateKeyExW failed: {r:?}"));
        }
        let r = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&bytes));
        let _ = RegCloseKey(key);
        if r != ERROR_SUCCESS {
            return Err(format!("RegSetValueExW failed: {r:?}"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn unregister() -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{RegDeleteKeyW, HKEY_CURRENT_USER};
    let path = key_path();
    let r = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) };
    if r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("RegDeleteKeyW failed: {r:?}"))
    }
}

#[cfg(windows)]
fn main() {
    let arg = std::env::args().nth(1);
    let Some(mode) = parse_mode(arg.as_deref()) else {
        eprintln!("usage: toast_spike --powershell | --app-id | --register | --unregister");
        std::process::exit(2);
    };
    let (what, result) = match mode {
        Mode::PowerShell => (
            "toast with the PowerShell app id",
            show(tauri_winrt_notification::Toast::POWERSHELL_APP_ID),
        ),
        Mode::AppId => ("toast with app id NetLantern.NetLantern", show(APP_ID)),
        Mode::Register => ("wrote HKCU AppUserModelId key", register()),
        Mode::Unregister => ("deleted HKCU AppUserModelId key", unregister()),
    };
    match result {
        Ok(()) => println!("ok: {what}"),
        Err(e) => {
            eprintln!("error: {what}: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("toast_spike only runs on Windows");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_arguments() {
        assert_eq!(parse_mode(Some("--powershell")), Some(Mode::PowerShell));
        assert_eq!(parse_mode(Some("--app-id")), Some(Mode::AppId));
        assert_eq!(parse_mode(Some("--register")), Some(Mode::Register));
        assert_eq!(parse_mode(Some("--unregister")), Some(Mode::Unregister));
    }

    #[test]
    fn rejects_unknown_or_missing() {
        assert_eq!(parse_mode(None), None);
        assert_eq!(parse_mode(Some("--nope")), None);
    }
}
