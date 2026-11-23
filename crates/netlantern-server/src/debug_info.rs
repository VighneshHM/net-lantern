//! The "copy debug info" text: version, Windows build, network state, recent
//! problems and the latest log lines, with everything private blanked out so
//! the user can paste it into a bug report.
//!
//! Redaction runs on the whole assembled text, headers included. The login
//! URL, tokens and session ids are never put into the text in the first place.

use std::borrow::Cow;

use netlantern_core::{CoreError, MonitorState};
use netlantern_platform::{InterfaceKind, NetworkCategory, PlatformError};
use regex::{Regex, RegexBuilder};

use crate::error::ApiError;
use crate::phone::PhoneState;
use crate::state::{AppState, HostKind};
use netlantern_platform::FirewallVerdict;

/// Problems and log lines kept in the text, newest last.
const MAX_PROBLEMS: usize = 50;
const MAX_LINES: usize = 200;
/// Names per regex: keeps each pattern far below the regex size limit.
const NAMES_PER_REGEX: usize = 256;

/// Addresses that are not private and help to read the log.
const PUBLIC_IPS: [&str; 3] = ["1.1.1.1", "8.8.8.8", "127.0.0.1"];

/// Blanks out private values in text. Build it with every name the store
/// knows; see [`Redactor::apply`] for the rules.
pub struct Redactor {
    /// Longest names first; each regex holds a slice of them.
    names: Vec<Regex>,
    user_dir: Option<Regex>,
    mac: Regex,
    ipv4: Regex,
    ipv6: Regex,
}

impl Redactor {
    /// `names`: every device name, hostname, announced name and SSID label.
    /// `user_dir`: the user's home folder, replaced by `%USERPROFILE%`.
    pub fn new(names: Vec<String>, user_dir: Option<String>) -> Self {
        let mut names: Vec<String> = names
            .into_iter()
            .map(|n| n.trim().to_string())
            .filter(|n| n.chars().count() >= 2)
            .collect();
        // Longest first, so "Lounge TV" goes before "TV".
        names.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        names.dedup_by(|a, b| a.to_lowercase() == b.to_lowercase());
        let names = names
            .chunks(NAMES_PER_REGEX)
            .filter_map(|chunk| {
                let pattern = chunk
                    .iter()
                    .map(|n| regex::escape(n))
                    .collect::<Vec<_>>()
                    .join("|");
                RegexBuilder::new(&pattern)
                    .case_insensitive(true)
                    .size_limit(64 << 20)
                    .build()
                    .ok()
            })
            .collect();

        let user_dir = user_dir
            .map(|d| d.trim().trim_end_matches(['\\', '/']).to_string())
            .filter(|d| !d.is_empty())
            .map(|d| {
                // As written, with forward slashes, and with doubled
                // backslashes (how a debug-formatted path reads in a log).
                let mut forms = vec![d.replace('\\', "\\\\"), d.clone(), d.replace('\\', "/")];
                forms.dedup();
                let pattern = forms
                    .iter()
                    .map(|f| regex::escape(f))
                    .collect::<Vec<_>>()
                    .join("|");
                RegexBuilder::new(&pattern)
                    .case_insensitive(true)
                    .build()
                    .expect("an escaped literal is a valid regex")
            });

        Self {
            names,
            user_dir,
            mac: Regex::new(
                r"(?i)\b(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\b|\b(?:[0-9a-f]{2}-){5}[0-9a-f]{2}\b",
            )
            .expect("valid regex"),
            ipv4: Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b").expect("valid regex"),
            // Full form, or anything with a `::`. A bare `::` or a Rust path
            // like `a::b` has no digit and is left alone (see `apply`), and a
            // clock time like 12:34:56 has no `::` and too few groups.
            ipv6: Regex::new(
                r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b|(?:\b[0-9a-f]{1,4}(?::[0-9a-f]{1,4})*)?::(?:[0-9a-f]{1,4}(?::[0-9a-f]{1,4})*\b)?",
            )
            .expect("valid regex"),
        }
    }

    /// IPv4 addresses (except 1.1.1.1, 8.8.8.8 and 127.0.0.1) become
    /// `x.x.x.x`, IPv6 addresses `x::x`, MACs `xx:xx:xx:xx:xx:xx`, each known
    /// name (2+ characters, case-insensitive, literal, longest first)
    /// `[name]`, and the user's folder `%USERPROFILE%`.
    pub fn apply(&self, text: &str) -> String {
        let text = self.mac.replace_all(text, "xx:xx:xx:xx:xx:xx");
        let text = self.ipv4.replace_all(&text, |c: &regex::Captures| {
            let ip = &c[0];
            let octets_ok = ip
                .split('.')
                .all(|part| part.parse::<u16>().is_ok_and(|n| n <= 255));
            if PUBLIC_IPS.contains(&ip) || !octets_ok {
                ip.to_string()
            } else {
                "x.x.x.x".to_string()
            }
        });
        let text = self.ipv6.replace_all(&text, |c: &regex::Captures| {
            let found = &c[0];
            if found.chars().any(|ch| ch.is_ascii_digit()) {
                "x::x".to_string()
            } else {
                found.to_string()
            }
        });
        let mut text: Cow<str> = text;
        if let Some(dir) = &self.user_dir {
            text = Cow::Owned(dir.replace_all(&text, "%USERPROFILE%").into_owned());
        }
        for names in &self.names {
            text = Cow::Owned(names.replace_all(&text, "[name]").into_owned());
        }
        text.into_owned()
    }
}

/// The home folder: `USERPROFILE`, else the parent of the `AppData` folder
/// `LOCALAPPDATA` sits in.
pub fn user_dir_from(userprofile: Option<&str>, local_app_data: Option<&str>) -> Option<String> {
    if let Some(dir) = userprofile.filter(|d| !d.is_empty()) {
        return Some(dir.to_string());
    }
    let local = local_app_data.filter(|d| !d.is_empty())?;
    let path = std::path::Path::new(local);
    let app_data = path.ancestors().find(|p| {
        p.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("AppData"))
    })?;
    app_data.parent().map(|p| p.display().to_string())
}

fn user_dir() -> Option<String> {
    user_dir_from(
        std::env::var("USERPROFILE").ok().as_deref(),
        std::env::var("LOCALAPPDATA").ok().as_deref(),
    )
}

/// The newest `max` items, in the order given.
fn tail(items: Vec<String>, max: usize) -> Vec<String> {
    let skip = items.len().saturating_sub(max);
    items.into_iter().skip(skip).collect()
}

/// Everything the text says, before redaction.
struct Facts {
    version: &'static str,
    host: HostKind,
    os: String,
    interface: &'static str,
    wifi_details: &'static str,
    category: &'static str,
    phone: String,
    monitoring: &'static str,
    problems: Vec<String>,
    lines: Vec<String>,
}

impl Facts {
    fn render(&self) -> String {
        let host = match self.host {
            HostKind::Exe => "exe",
            HostKind::Desktop => "desktop app",
        };
        let mut out = format!(
            "Net Lantern {} · {host}\nOS: {}\nInterface: {}\nWi-Fi details: {}\n\
             Network category: {}\nPhone access: {}\nMonitoring: {}\n",
            self.version,
            self.os,
            self.interface,
            self.wifi_details,
            self.category,
            self.phone,
            self.monitoring,
        );
        for (title, items) in [
            ("Recent problems", &self.problems),
            ("Last log lines", &self.lines),
        ] {
            out.push_str(&format!("\n{title}:\n"));
            if items.is_empty() {
                out.push_str("(none)\n");
            }
            for line in items {
                out.push_str(line.trim_end());
                out.push('\n');
            }
        }
        out
    }
}

async fn gather(state: &AppState) -> Facts {
    let interface = match state.platform.active_interface().await {
        Ok(i) => match i.kind {
            InterfaceKind::Wifi => "wifi",
            InterfaceKind::Ethernet => "ethernet",
            InterfaceKind::Other => "other",
        },
        Err(PlatformError::NotConnected) => "not connected",
        Err(_) => "unknown",
    };
    let wifi_details = match state.platform.wifi_current().await {
        Ok(Some(_)) => "shown",
        Err(PlatformError::PermissionNeeded { .. }) => "hidden (location permission)",
        _ => "no Wi-Fi",
    };
    let category = match state.platform.network_category().await {
        Ok(NetworkCategory::Private) => "Private",
        Ok(NetworkCategory::Public) => "Public",
        Ok(NetworkCategory::Domain) => "Domain",
        Err(_) => "unknown",
    };
    let phone = match &state.phone {
        None => "off".to_string(),
        Some(phone) => {
            let status = phone.status().await;
            let firewall = match status.firewall {
                Some(FirewallVerdict::Allowed) => "allowed",
                Some(FirewallVerdict::Blocked) => "blocked",
                Some(FirewallVerdict::NotDecided) => "not decided",
                Some(FirewallVerdict::Off) => "off",
                None => "unknown",
            };
            let access = match (status.state, status.reason) {
                (PhoneState::Off, _) => "off".to_string(),
                (PhoneState::On, _) => "on".to_string(),
                (PhoneState::Paused, Some(reason)) => format!("paused ({reason})"),
                (PhoneState::Paused, None) => "paused".to_string(),
            };
            format!("{access} · firewall {firewall}")
        }
    };
    let monitoring = match state.monitor.status().state {
        MonitorState::Monitoring => "monitoring",
        MonitorState::WaitingForScan => "waiting for a scan",
        MonitorState::NotConnected => "not connected",
    };
    let (problems, lines) = match &state.logs {
        Some(taps) => ((taps.problems)(), (taps.lines)()),
        None => (Vec::new(), Vec::new()),
    };
    Facts {
        version: state.host.version,
        host: state.host.kind,
        os: state.platform.os_description().await,
        interface,
        wifi_details,
        category,
        phone,
        monitoring,
        problems: tail(problems, MAX_PROBLEMS),
        lines: tail(lines, MAX_LINES),
    }
}

/// The redacted debug text. Fails, rather than risk an unredacted answer,
/// when the store's names cannot be read.
pub async fn build(state: &AppState) -> Result<String, ApiError> {
    let names = state.store.known_names().await.map_err(CoreError::from)?;
    let redactor = Redactor::new(names, user_dir());
    Ok(redactor.apply(&gather(state).await.render()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_removes_names_addresses_ssids_and_the_user_folder() {
        let redactor = Redactor::new(
            vec!["Lounge TV".into(), "Home (5G)+".into()],
            Some(r"C:\Users\alice".into()),
        );
        let input = "Lounge TV (192.168.1.52) aa-bb-cc-dd-ee-ff\n\
                     fe80::1c2:3ff:fe4a:5b6c\n\
                     joined home (5g)+ ok\n\
                     C:\\Users\\alice\\AppData\\Local\\NetLantern\\logs\n\
                     pinged 1.1.1.1";
        let out = redactor.apply(input);
        for private in [
            "Lounge",
            "192.168.1.52",
            "aa-bb-cc-dd-ee-ff",
            "1c2",
            "5G",
            "5g",
            "alice",
        ] {
            assert!(!out.contains(private), "{private} leaked: {out}");
        }
        assert!(out.contains("pinged 1.1.1.1"), "{out}");
        for shown in [
            "[name]",
            "x.x.x.x",
            "xx:xx:xx:xx:xx:xx",
            "x::x",
            r"%USERPROFILE%\AppData",
        ] {
            assert!(out.contains(shown), "{shown} missing: {out}");
        }
    }

    #[test]
    fn longest_name_wins() {
        let redactor = Redactor::new(vec!["TV".into(), "Lounge TV".into()], None);
        assert_eq!(
            redactor.apply("the Lounge TV and the tv"),
            "the [name] and the [name]"
        );
    }

    #[test]
    fn short_names_are_not_redacted() {
        let redactor = Redactor::new(vec!["a".into(), " ".into(), "".into()], None);
        assert_eq!(redactor.apply("a lazy cat"), "a lazy cat");
    }

    #[test]
    fn public_addresses_mac_forms_and_clock_times() {
        let redactor = Redactor::new(vec![], None);
        let out = redactor.apply(
            "8.8.8.8 127.0.0.1 10.0.0.1 AA:BB:CC:DD:EE:FF 12:34:56 netlantern_server::phone 1.2.3",
        );
        assert_eq!(
            out,
            "8.8.8.8 127.0.0.1 x.x.x.x xx:xx:xx:xx:xx:xx 12:34:56 netlantern_server::phone 1.2.3"
        );
        assert_eq!(
            redactor.apply("::1 and 2001:db8::ff00:42:8329"),
            "x::x and x::x"
        );
    }

    #[test]
    fn the_user_folder_is_found_in_every_spelling() {
        let redactor = Redactor::new(vec![], Some(r"C:\Users\alice".into()));
        assert_eq!(
            redactor.apply(r#"c:\users\ALICE\x C:/Users/alice/y "C:\\Users\\alice\\z""#),
            r#"%USERPROFILE%\x %USERPROFILE%/y "%USERPROFILE%\\z""#
        );
    }

    #[test]
    fn the_user_dir_comes_from_userprofile_or_local_app_data() {
        assert_eq!(
            user_dir_from(Some(r"C:\Users\bob"), None).as_deref(),
            Some(r"C:\Users\bob")
        );
        assert_eq!(
            user_dir_from(None, Some(r"C:\Users\bob\AppData\Local")).as_deref(),
            Some(r"C:\Users\bob")
        );
        assert_eq!(user_dir_from(Some(""), Some("")), None);
    }

    #[test]
    fn only_the_newest_lines_are_kept() {
        let lines: Vec<String> = (0..5).map(|n| n.to_string()).collect();
        assert_eq!(tail(lines, 2), vec!["3", "4"]);
    }

    #[test]
    fn the_text_has_the_documented_layout() {
        let facts = Facts {
            version: "1.2.3",
            host: HostKind::Desktop,
            os: "Windows 11 (build 26200)".into(),
            interface: "wifi",
            wifi_details: "shown",
            category: "Private",
            phone: "paused (public_network) · firewall unknown".into(),
            monitoring: "monitoring",
            problems: vec!["WARN a".into()],
            lines: vec![],
        };
        assert_eq!(
            facts.render(),
            "Net Lantern 1.2.3 · desktop app\nOS: Windows 11 (build 26200)\nInterface: wifi\n\
             Wi-Fi details: shown\nNetwork category: Private\n\
             Phone access: paused (public_network) · firewall unknown\nMonitoring: monitoring\n\
             \nRecent problems:\nWARN a\n\nLast log lines:\n(none)\n"
        );
    }
}
