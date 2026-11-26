//! Start at login through our own value under `HKCU\...\Run`.
//!
//! The value holds the quoted exe path plus its arguments. Task Manager's
//! Startup tab keeps its own switch under `StartupApproved\Run`: a binary
//! value whose first byte is odd when the entry is disabled there. Reading
//! respects that switch; turning start at login on clears it.
//!
//! Only the current user's hive is touched, never HKLM.

use std::io;
use std::path::Path;

#[cfg(windows)]
pub use hkcu::HkcuRegistry;

/// The key Windows runs entries from at login, under HKCU.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Task Manager's per-entry enabled/disabled switch, under HKCU.
pub const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
/// Our value's name under both keys.
pub const VALUE_NAME: &str = "Net Lantern";

/// The command line for the `Run` value: the exe path in double quotes (it may
/// hold spaces), then each argument separated by a space.
pub fn run_command(exe: &Path, args: &[&str]) -> String {
    let mut command = format!("\"{}\"", exe.display());
    for arg in args {
        command.push(' ');
        command.push_str(arg);
    }
    command
}

/// Values under HKEY_CURRENT_USER. A missing key or value reads as `Ok(None)`
/// and deletes as a no-op.
pub trait UserRegistry: Send + Sync {
    fn get_string(&self, key: &str, name: &str) -> io::Result<Option<String>>;
    fn get_binary(&self, key: &str, name: &str) -> io::Result<Option<Vec<u8>>>;
    fn set_string(&self, key: &str, name: &str, value: &str) -> io::Result<()>;
    fn delete(&self, key: &str, name: &str) -> io::Result<()>;
}

/// The start-at-login switch over a registry and the command to run.
pub struct StartAtLogin<R: UserRegistry> {
    registry: R,
    command: String,
    exe_exists: fn(&Path) -> bool,
}

/// The exe a `Run` command starts, parsed the way Windows does: the text
/// inside the first pair of quotes when it starts with a quote, otherwise
/// everything up to and including the first `.exe` (any case). `None` when
/// neither applies.
fn stored_exe(command: &str) -> Option<String> {
    if let Some(rest) = command.strip_prefix('"') {
        return rest.find('"').map(|end| rest[..end].to_string());
    }
    // ASCII lowercasing keeps byte offsets, so `end` indexes `command`.
    let end = command.to_ascii_lowercase().find(".exe")? + ".exe".len();
    Some(command[..end].to_string())
}

fn same_path(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl<R: UserRegistry> StartAtLogin<R> {
    pub fn new(registry: R, command: String) -> Self {
        Self {
            registry,
            command,
            exe_exists: Path::is_file,
        }
    }

    /// Replaces the check `repair` uses to see whether a stored exe still
    /// exists (tests pass one that touches no files).
    pub fn with_exe_check(mut self, exe_exists: fn(&Path) -> bool) -> Self {
        self.exe_exists = exe_exists;
        self
    }

    /// On when our `Run` value exists and Task Manager has not disabled it.
    pub fn is_enabled(&self) -> io::Result<bool> {
        if self.registry.get_string(RUN_KEY, VALUE_NAME)?.is_none() {
            return Ok(false);
        }
        let disabled = self
            .registry
            .get_binary(APPROVED_KEY, VALUE_NAME)?
            .and_then(|bytes| bytes.first().copied())
            .is_some_and(|first| first & 1 == 1);
        Ok(!disabled)
    }

    /// On writes our command and clears Task Manager's switch; off removes
    /// both values.
    pub fn set_enabled(&self, on: bool) -> io::Result<()> {
        if on {
            // The Run value goes first, so a failed write leaves everything
            // as it was, Task Manager's switch included; the switch is only
            // cleared once there is an entry for it to apply to.
            self.registry
                .set_string(RUN_KEY, VALUE_NAME, &self.command)?;
        } else {
            self.registry.delete(RUN_KEY, VALUE_NAME)?;
        }
        self.registry.delete(APPROVED_KEY, VALUE_NAME)
    }

    /// Points an existing `Run` value at this command when it is ours to
    /// fix: it names this same exe (an older build wrote it unquoted), or the
    /// exe it names is gone (moved) or cannot be parsed. A live entry for
    /// another exe, such as the installed copy while a dev build runs, is
    /// left alone. A missing value stays missing, and Task Manager's switch
    /// is left as it is.
    pub fn repair(&self) -> io::Result<()> {
        let Some(current) = self.registry.get_string(RUN_KEY, VALUE_NAME)? else {
            return Ok(());
        };
        if current == self.command {
            return Ok(());
        }
        let ours = match (stored_exe(&current), stored_exe(&self.command)) {
            (Some(stored), Some(this)) => same_path(&stored, &this),
            _ => false,
        };
        let gone = stored_exe(&current).is_none_or(|exe| !(self.exe_exists)(Path::new(&exe)));
        if ours || gone {
            self.registry.set_string(RUN_KEY, VALUE_NAME, &self.command)
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
mod hkcu {
    use std::io;

    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ,
        REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
    };

    use super::UserRegistry;

    /// The real HKEY_CURRENT_USER.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct HkcuRegistry;

    fn os_error(code: WIN32_ERROR) -> io::Error {
        io::Error::from_raw_os_error(code.0 as i32)
    }

    /// An open key, closed on drop.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful open or create and is
            // closed only here.
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    /// Opens `key` for reading; `Ok(None)` when it does not exist.
    fn open_for_read(key: &str) -> io::Result<Option<Key>> {
        let mut handle = HKEY::default();
        // SAFETY: the subkey is a NUL-terminated HSTRING that outlives the
        // call and `handle` is a valid out-pointer.
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(key),
                None,
                KEY_QUERY_VALUE,
                &mut handle,
            )
        };
        match rc {
            r if r.is_ok() => Ok(Some(Key(handle))),
            ERROR_FILE_NOT_FOUND => Ok(None),
            r => Err(os_error(r)),
        }
    }

    /// The raw bytes and type of `name` under `key`, or `None` when either
    /// is missing.
    fn query(key: &str, name: &str) -> io::Result<Option<(REG_VALUE_TYPE, Vec<u8>)>> {
        let Some(key) = open_for_read(key)? else {
            return Ok(None);
        };
        let name = HSTRING::from(name);
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let mut kind = REG_VALUE_TYPE::default();
            let mut len = buf.len() as u32;
            let data = if buf.is_empty() {
                None
            } else {
                Some(buf.as_mut_ptr())
            };
            // SAFETY: `key.0` is open; `name` is NUL-terminated; `data`, when
            // given, points at `len` writable bytes.
            let rc = unsafe {
                RegQueryValueExW(key.0, &name, None, Some(&mut kind), data, Some(&mut len))
            };
            match rc {
                // A size query (no buffer) succeeds with the length needed.
                r if r.is_ok() && data.is_none() && len > 0 => buf = vec![0; len as usize],
                r if r.is_ok() => {
                    buf.truncate(len as usize);
                    return Ok(Some((kind, buf)));
                }
                // The value grew between the calls.
                ERROR_MORE_DATA => buf = vec![0; len as usize],
                ERROR_FILE_NOT_FOUND => return Ok(None),
                r => return Err(os_error(r)),
            }
        }
    }

    /// A REG_SZ value's bytes as text, up to the first NUL.
    fn wide(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    }

    impl UserRegistry for HkcuRegistry {
        /// REG_EXPAND_SZ is returned as stored, `%VAR%` references not
        /// expanded: the value is only compared with our own command.
        fn get_string(&self, key: &str, name: &str) -> io::Result<Option<String>> {
            match query(key, name)? {
                None => Ok(None),
                Some((kind, bytes)) if kind == REG_SZ || kind == REG_EXPAND_SZ => {
                    Ok(Some(wide(&bytes)))
                }
                Some(_) => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the registry value is not a string",
                )),
            }
        }

        fn get_binary(&self, key: &str, name: &str) -> io::Result<Option<Vec<u8>>> {
            Ok(query(key, name)?.map(|(_, bytes)| bytes))
        }

        fn set_string(&self, key: &str, name: &str, value: &str) -> io::Result<()> {
            let mut handle = HKEY::default();
            // SAFETY: the subkey is a NUL-terminated HSTRING that outlives the
            // call and `handle` is a valid out-pointer.
            let rc = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(key),
                    None,
                    PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_SET_VALUE,
                    None,
                    &mut handle,
                    None,
                )
            };
            if rc.is_err() {
                return Err(os_error(rc));
            }
            let key = Key(handle);
            let bytes: Vec<u8> = value
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect();
            // SAFETY: `key.0` is open for setting values; `name` is
            // NUL-terminated; the data is a NUL-terminated UTF-16 string.
            let rc =
                unsafe { RegSetValueExW(key.0, &HSTRING::from(name), None, REG_SZ, Some(&bytes)) };
            if rc.is_err() {
                return Err(os_error(rc));
            }
            Ok(())
        }

        fn delete(&self, key: &str, name: &str) -> io::Result<()> {
            // SAFETY: both strings are NUL-terminated HSTRINGs that outlive
            // the call.
            let rc = unsafe {
                RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name))
            };
            match rc {
                r if r.is_ok() => Ok(()),
                ERROR_FILE_NOT_FOUND => Ok(()),
                r => Err(os_error(r)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Clone, Debug, PartialEq)]
    enum Value {
        Str(String),
        Bin(Vec<u8>),
    }

    #[derive(Default)]
    struct FakeRegistry {
        values: Mutex<HashMap<(String, String), Value>>,
        fail_reads: bool,
    }

    impl FakeRegistry {
        fn with(entries: &[(&str, &str, Value)]) -> Self {
            let fake = Self::default();
            for (key, name, value) in entries {
                fake.values
                    .lock()
                    .unwrap()
                    .insert((key.to_string(), name.to_string()), value.clone());
            }
            fake
        }

        fn get(&self, key: &str, name: &str) -> Option<Value> {
            self.values
                .lock()
                .unwrap()
                .get(&(key.to_string(), name.to_string()))
                .cloned()
        }
    }

    impl UserRegistry for FakeRegistry {
        fn get_string(&self, key: &str, name: &str) -> io::Result<Option<String>> {
            if self.fail_reads {
                return Err(io::Error::other("access denied"));
            }
            Ok(match self.get(key, name) {
                Some(Value::Str(s)) => Some(s),
                _ => None,
            })
        }

        fn get_binary(&self, key: &str, name: &str) -> io::Result<Option<Vec<u8>>> {
            if self.fail_reads {
                return Err(io::Error::other("access denied"));
            }
            Ok(match self.get(key, name) {
                Some(Value::Bin(b)) => Some(b),
                _ => None,
            })
        }

        fn set_string(&self, key: &str, name: &str, value: &str) -> io::Result<()> {
            self.values.lock().unwrap().insert(
                (key.to_string(), name.to_string()),
                Value::Str(value.to_string()),
            );
            Ok(())
        }

        fn delete(&self, key: &str, name: &str) -> io::Result<()> {
            self.values
                .lock()
                .unwrap()
                .remove(&(key.to_string(), name.to_string()));
            Ok(())
        }
    }

    const CMD: &str = "\"C:\\Net Lantern\\Net Lantern.exe\" --hidden";

    /// Every exe counts as missing unless a test says otherwise: no real
    /// file checks.
    fn start(fake: FakeRegistry) -> StartAtLogin<FakeRegistry> {
        StartAtLogin::new(fake, CMD.to_string()).with_exe_check(|_| false)
    }

    fn approved(first: u8) -> Value {
        let mut bytes = vec![0u8; 12];
        bytes[0] = first;
        Value::Bin(bytes)
    }

    #[test]
    fn run_command_quotes_the_path() {
        assert_eq!(
            run_command(
                Path::new(r"C:\Users\a b\AppData\Local\Net Lantern\Net Lantern.exe"),
                &["--hidden"]
            ),
            "\"C:\\Users\\a b\\AppData\\Local\\Net Lantern\\Net Lantern.exe\" --hidden"
        );
    }

    #[test]
    fn off_when_no_value() {
        assert!(!start(FakeRegistry::default()).is_enabled().unwrap());
    }

    #[test]
    fn on_after_enable_and_writes_the_command() {
        let s = start(FakeRegistry::default());
        s.set_enabled(true).unwrap();
        assert!(s.is_enabled().unwrap());
        assert_eq!(
            s.registry.get(RUN_KEY, VALUE_NAME),
            Some(Value::Str(CMD.to_string()))
        );
    }

    #[test]
    fn off_after_disable_and_both_values_are_gone() {
        let s = start(FakeRegistry::with(&[
            (RUN_KEY, VALUE_NAME, Value::Str(CMD.to_string())),
            (APPROVED_KEY, VALUE_NAME, approved(2)),
        ]));
        s.set_enabled(false).unwrap();
        assert!(!s.is_enabled().unwrap());
        assert_eq!(s.registry.get(RUN_KEY, VALUE_NAME), None);
        assert_eq!(s.registry.get(APPROVED_KEY, VALUE_NAME), None);
    }

    #[test]
    fn a_startup_disabled_in_task_manager_reads_off() {
        let s = start(FakeRegistry::with(&[
            (RUN_KEY, VALUE_NAME, Value::Str(CMD.to_string())),
            (APPROVED_KEY, VALUE_NAME, approved(3)),
        ]));
        assert!(!s.is_enabled().unwrap());
    }

    #[test]
    fn an_approved_entry_marked_enabled_reads_on() {
        let s = start(FakeRegistry::with(&[
            (RUN_KEY, VALUE_NAME, Value::Str(CMD.to_string())),
            (APPROVED_KEY, VALUE_NAME, approved(2)),
        ]));
        assert!(s.is_enabled().unwrap());
    }

    #[test]
    fn turning_on_clears_the_task_manager_flag() {
        let s = start(FakeRegistry::with(&[
            (RUN_KEY, VALUE_NAME, Value::Str(CMD.to_string())),
            (APPROVED_KEY, VALUE_NAME, approved(3)),
        ]));
        s.set_enabled(true).unwrap();
        assert_eq!(s.registry.get(APPROVED_KEY, VALUE_NAME), None);
        assert!(s.is_enabled().unwrap());
    }

    #[test]
    fn repair_rewrites_a_stale_command() {
        let s = start(FakeRegistry::with(&[(
            RUN_KEY,
            VALUE_NAME,
            Value::Str(r"C:\Old Place\Net Lantern.exe --hidden".to_string()),
        )]));
        s.repair().unwrap();
        assert_eq!(
            s.registry.get(RUN_KEY, VALUE_NAME),
            Some(Value::Str(CMD.to_string()))
        );
    }

    #[test]
    fn repair_leaves_a_live_entry_for_another_exe_alone() {
        let other = r#""D:\Installed\Net Lantern.exe" --hidden"#;
        let s = start(FakeRegistry::with(&[(
            RUN_KEY,
            VALUE_NAME,
            Value::Str(other.to_string()),
        )]))
        .with_exe_check(|exe| exe == Path::new(r"D:\Installed\Net Lantern.exe"));
        s.repair().unwrap();
        assert_eq!(
            s.registry.get(RUN_KEY, VALUE_NAME),
            Some(Value::Str(other.to_string()))
        );
    }

    #[test]
    fn repair_repoints_an_entry_whose_exe_is_gone() {
        let s = start(FakeRegistry::with(&[(
            RUN_KEY,
            VALUE_NAME,
            Value::Str(r#""D:\Gone\Net Lantern.exe" --hidden"#.to_string()),
        )]))
        .with_exe_check(|exe| exe != Path::new(r"D:\Gone\Net Lantern.exe"));
        s.repair().unwrap();
        assert_eq!(
            s.registry.get(RUN_KEY, VALUE_NAME),
            Some(Value::Str(CMD.to_string()))
        );
    }

    #[test]
    fn repair_quotes_an_unquoted_entry_for_this_exe() {
        // The plugin's unquoted value, in another case: this exe still exists.
        let s = start(FakeRegistry::with(&[(
            RUN_KEY,
            VALUE_NAME,
            Value::Str(r"c:\net lantern\NET LANTERN.EXE --hidden".to_string()),
        )]))
        .with_exe_check(|_| true);
        s.repair().unwrap();
        assert_eq!(
            s.registry.get(RUN_KEY, VALUE_NAME),
            Some(Value::Str(CMD.to_string()))
        );
    }

    #[test]
    fn repair_rewrites_an_unparsable_entry() {
        for stored in ["\"C:\\no closing quote.exe --hidden", "something else"] {
            let s = start(FakeRegistry::with(&[(
                RUN_KEY,
                VALUE_NAME,
                Value::Str(stored.to_string()),
            )]))
            .with_exe_check(|_| true);
            s.repair().unwrap();
            assert_eq!(
                s.registry.get(RUN_KEY, VALUE_NAME),
                Some(Value::Str(CMD.to_string())),
                "{stored}"
            );
        }
    }

    #[test]
    fn stored_exe_follows_the_windows_rules() {
        assert_eq!(
            stored_exe(r#""C:\a b\x.exe" --hidden"#).as_deref(),
            Some(r"C:\a b\x.exe")
        );
        assert_eq!(
            stored_exe(r"C:\a b\X.EXE --hidden").as_deref(),
            Some(r"C:\a b\X.EXE")
        );
        assert_eq!(stored_exe("\"C:\\open"), None);
        assert_eq!(stored_exe("C:\\no extension"), None);
    }

    #[test]
    fn an_empty_approved_value_reads_on() {
        let s = start(FakeRegistry::with(&[
            (RUN_KEY, VALUE_NAME, Value::Str(CMD.to_string())),
            (APPROVED_KEY, VALUE_NAME, Value::Bin(Vec::new())),
        ]));
        assert!(s.is_enabled().unwrap());
    }

    #[test]
    fn no_approved_value_reads_on() {
        let s = start(FakeRegistry::with(&[(
            RUN_KEY,
            VALUE_NAME,
            Value::Str(CMD.to_string()),
        )]));
        assert!(s.is_enabled().unwrap());
    }

    #[test]
    fn repair_leaves_a_missing_value_missing() {
        let s = start(FakeRegistry::default());
        s.repair().unwrap();
        assert_eq!(s.registry.get(RUN_KEY, VALUE_NAME), None);
    }

    #[test]
    fn a_registry_error_is_returned_not_swallowed() {
        let s = start(FakeRegistry {
            fail_reads: true,
            ..FakeRegistry::default()
        });
        assert!(s.is_enabled().is_err());
    }
}
