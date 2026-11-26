//! The "copy debug info" text: version, Windows build, network state, recent
//! problems and the latest log lines, with everything private blanked out so
//! the user can paste it into a bug report.
//!
//! Redaction runs on the whole assembled text, headers included. The login
//! URL, tokens and session ids are never put into the text in the first place.

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
/// Names per regex, before any halving.
const NAMES_PER_REGEX: usize = 64;
/// Names longer than this (in characters) are replaced as exact text rather
/// than through a regex, so a long name can never hit the regex size limit.
const MAX_REGEX_NAME_CHARS: usize = 120;
/// The compiled size limit of one names regex.
const REGEX_SIZE_LIMIT: usize = 32 << 20;

/// Addresses that are not private and help to read the log.
const PUBLIC_IPS: [&str; 3] = ["1.1.1.1", "8.8.8.8", "127.0.0.1"];

/// Profile folders under `\Users` that belong to no person.
const SYSTEM_PROFILES: [&str; 4] = ["public", "default", "default user", "all users"];

/// A redactor that could not be built. The debug info is then not given at
/// all: text must never leave on a path where a known name was not applied.
#[derive(Debug, PartialEq, Eq)]
pub struct RedactorError(String);

/// Blanks out private values in text. Build it with every name the store
/// knows; see [`Redactor::apply`] for the rules.
pub struct Redactor {
    /// Names over `MAX_REGEX_NAME_CHARS`: one regex each, longest first.
    long_names: Vec<Regex>,
    /// Longest names first; each regex holds a slice of them.
    names: Vec<Regex>,
    user_dir: Vec<Regex>,
    profile: Regex,
    /// Launch or phone tokens in URLs, and the session cookie value.
    tokens: Vec<Regex>,
    /// A whole cookie header.
    cookie: Regex,
    mac: Vec<Regex>,
    ipv4: Regex,
    ipv6: Regex,
}

/// Compiles `names` (escaped, case-insensitive) into regexes in order,
/// halving a chunk that does not compile. Fails if one name alone does not.
fn compile_names(
    names: &[String],
    size_limit: usize,
    out: &mut Vec<Regex>,
) -> Result<(), RedactorError> {
    let pattern = names
        .iter()
        .map(|n| regex::escape(n))
        .collect::<Vec<_>>()
        .join("|");
    match RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .size_limit(size_limit)
        .build()
    {
        Ok(regex) => {
            out.push(regex);
            Ok(())
        }
        Err(e) if names.len() == 1 => Err(RedactorError(e.to_string())),
        Err(_) => {
            let (head, tail) = names.split_at(names.len() / 2);
            compile_names(head, size_limit, out)?;
            compile_names(tail, size_limit, out)
        }
    }
}

fn fixed(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid regex")
}

impl Redactor {
    /// `names`: every device name, hostname, announced name and SSID label
    /// (and the PC's own names). `user_dir`: the user's home folder, replaced
    /// by `%USERPROFILE%`. Fails if any name cannot be made into a rule.
    pub fn new(names: Vec<String>, user_dir: Option<String>) -> Result<Self, RedactorError> {
        Self::with_size_limit(names, user_dir, REGEX_SIZE_LIMIT)
    }

    fn with_size_limit(
        names: Vec<String>,
        user_dir: Option<String>,
        size_limit: usize,
    ) -> Result<Self, RedactorError> {
        let mut names: Vec<String> = names
            .into_iter()
            .map(|n| n.trim().to_string())
            .filter(|n| n.chars().count() >= 2)
            .collect();
        // Longest first, so a regex's alternation prefers "Lounge TV" over
        // "TV". Order no longer decides what is hidden: overlapping matches
        // from different regexes merge into one span (see `apply`).
        names.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        names.dedup_by(|a, b| a.to_lowercase() == b.to_lowercase());
        let (long_names, short_names): (Vec<String>, Vec<String>) = names
            .into_iter()
            .partition(|n| n.chars().count() > MAX_REGEX_NAME_CHARS);
        let mut long_regexes = Vec::new();
        for name in &long_names {
            compile_names(std::slice::from_ref(name), size_limit, &mut long_regexes)?;
        }
        let mut names = Vec::new();
        for chunk in short_names.chunks(NAMES_PER_REGEX) {
            compile_names(chunk, size_limit, &mut names)?;
        }

        let mut user_dirs = Vec::new();
        if let Some(d) = user_dir
            .map(|d| d.trim().trim_end_matches(['\\', '/']).to_string())
            .filter(|d| !d.is_empty())
        {
            // As written, with forward slashes, and with doubled
            // backslashes (how a debug-formatted path reads in a log).
            let mut forms = vec![d.replace('\\', "\\\\"), d.clone(), d.replace('\\', "/")];
            forms.dedup();
            compile_names(&forms, size_limit, &mut user_dirs)?;
        }

        Ok(Self {
            long_names: long_regexes,
            names,
            user_dir: user_dirs,
            // Any other `\Users\<name>` profile folder, including the 8.3
            // short form (`ALICEW~1`), except the system profiles.
            profile: fixed(r#"(?i)(?:[a-z]:)?(?:\\\\|[\\/])Users(?:\\\\|[\\/])+[^\\/\s"'<>|:*?]+"#),
            tokens: vec![
                // A launch or phone token in a URL.
                fixed(r"(?i)[?&]t=[0-9a-f]{16,}"),
                fixed(r#"(?i)nl_session=[^\s;,"']*"#),
            ],
            cookie: fixed(r"(?i)\b(?:set-)?cookie:[^\r\n]*"),
            mac: vec![
                // Colons, hyphens or a mix of both.
                fixed(r"(?i)\b[0-9a-f]{2}(?:[:-][0-9a-f]{2}){5}\b"),
                // Cisco dotted.
                fixed(r"(?i)\b[0-9a-f]{4}\.[0-9a-f]{4}\.[0-9a-f]{4}\b"),
                // Twelve hex digits on their own (not inside a longer run).
                fixed(r"(?i)\b[0-9a-f]{12}\b"),
            ],
            // The address may touch a letter or underscore (`ip_10.0.0.1`),
            // but not a digit or dot (a version like 10.0.26200.1). The
            // captures hold the character before and any digits after.
            ipv4: fixed(r"(^|[^\d.])(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})(\d*)"),
            // Full form, or anything with a `::`. A bare `::` or a Rust path
            // like `a::b` has no digit and is left alone (see `apply`), and a
            // clock time like 12:34:56 has no `::` and too few groups.
            ipv6: fixed(
                r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b|(?:\b[0-9a-f]{1,4}(?::[0-9a-f]{1,4})*)?::(?:[0-9a-f]{1,4}(?::[0-9a-f]{1,4})*\b)?",
            ),
        })
    }

    /// Secrets (launch tokens in URLs, the session cookie) become
    /// `[redacted]`. MAC addresses (any separator, Cisco dotted, or 12 bare
    /// hex digits) become `xx:xx:xx:xx:xx:xx`; IPv4 addresses (except 1.1.1.1,
    /// 8.8.8.8 and 127.0.0.1) `x.x.x.x`; IPv6 addresses `x::x`; the user's
    /// folder `%USERPROFILE%`; and each known name (2+ characters,
    /// case-insensitive, literal) `[name]`.
    ///
    /// Every rule is matched on the original text (the address and cookie
    /// rules also on a copy with the names blanked out), and matches that
    /// overlap or touch are merged into one span with one placeholder, so a
    /// name can never cut an address open (or the reverse) and leave a piece
    /// behind. A span that one name match covers entirely is `[name]` (a
    /// name such as `cam-192.168.1.20-front`); otherwise the placeholder of
    /// the highest-ranked rule in it wins, in this order: cookie header >
    /// token (URL token or session) > MAC > IPv6 > IPv4 > user folder > name.
    pub fn apply(&self, text: &str) -> String {
        let mut spans: Vec<Span> = Vec::new();
        let add = |spans: &mut Vec<Span>, regex: &Regex, on: &str, rank: Rank| {
            spans.extend(
                regex
                    .find_iter(on)
                    .map(|m| Span::new(m.start(), m.end(), rank)),
            );
        };
        for names in self.long_names.iter().chain(&self.names) {
            add(&mut spans, names, text, Rank::Name);
        }
        // The rules that need a boundary before them (`\b`, or no digit or
        // dot before an IPv4 address) also run on a copy with every name
        // blanked to spaces, byte for byte: a name glued onto an address
        // (`Plug1aa:bb:cc:dd:ee:ff`, `Cam999.0.0.1`) would otherwise hide
        // it from them. Offsets are the same in both texts.
        let masked = mask(text, &spans);
        for on in [text, masked.as_str()] {
            add(&mut spans, &self.cookie, on, Rank::Cookie);
            for mac in &self.mac {
                add(&mut spans, mac, on, Rank::Mac);
            }
            self.find_ip_addresses(on, &mut spans);
        }
        for token in &self.tokens {
            add(&mut spans, token, text, Rank::Token);
        }
        for dir in &self.user_dir {
            add(&mut spans, dir, text, Rank::UserDir);
        }
        for m in self.profile.find_iter(text) {
            let segment = m
                .as_str()
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or("")
                .to_lowercase();
            if !SYSTEM_PROFILES.contains(&segment.as_str()) {
                spans.push(Span::new(m.start(), m.end(), Rank::UserDir));
            }
        }
        spans.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));

        let mut out = String::with_capacity(text.len());
        let mut done = 0;
        let mut i = 0;
        while i < spans.len() {
            // One merged span: everything that overlaps or touches.
            let (start, mut end) = (spans[i].start, spans[i].end);
            let mut j = i + 1;
            while j < spans.len() && spans[j].start <= end {
                end = end.max(spans[j].end);
                j += 1;
            }
            let group = &spans[i..j];
            let one_name_covers = group
                .iter()
                .any(|s| s.rank == Rank::Name && s.start == start && s.end == end);
            let rank = if one_name_covers {
                Rank::Name
            } else {
                group.iter().map(|s| s.rank).min().expect("non-empty")
            };
            out.push_str(&text[done..start]);
            out.push_str(rank.placeholder());
            done = end;
            i = j;
        }
        out.push_str(&text[done..]);
        out
    }

    /// IPv4 (private ones only) and IPv6 address spans in `text`.
    fn find_ip_addresses(&self, text: &str, spans: &mut Vec<Span>) {
        for c in self.ipv4.captures_iter(text) {
            let (ip, extra) = (c.get(2).expect("group 2"), &c[3]);
            let octets_ok = ip
                .as_str()
                .split('.')
                .all(|part| part.parse::<u16>().is_ok_and(|n| n <= 255));
            if octets_ok && extra.is_empty() && !PUBLIC_IPS.contains(&ip.as_str()) {
                spans.push(Span::new(ip.start(), ip.end(), Rank::Ipv4));
            }
        }
        for m in self.ipv6.find_iter(text) {
            // A bare `::` or a Rust path has no digit.
            if m.as_str().chars().any(|ch| ch.is_ascii_digit()) {
                spans.push(Span::new(m.start(), m.end(), Rank::Ipv6));
            }
        }
    }
}

/// `text` with the bytes of every span turned into ASCII spaces. Spans start
/// and end on character boundaries, so a multi-byte character becomes as
/// many spaces as it has bytes: the copy is valid UTF-8 of the same length.
fn mask(text: &str, spans: &[Span]) -> String {
    let mut bytes = text.as_bytes().to_vec();
    for s in spans {
        bytes[s.start..s.end].fill(b' ');
    }
    String::from_utf8(bytes).expect("whole characters were replaced")
}

/// What a merged span is replaced with, highest rank first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Rank {
    Cookie,
    Token,
    Mac,
    Ipv6,
    Ipv4,
    UserDir,
    Name,
}

impl Rank {
    fn placeholder(self) -> &'static str {
        match self {
            Rank::Cookie => "Cookie: [redacted]",
            Rank::Token => "[redacted]",
            Rank::Mac => "xx:xx:xx:xx:xx:xx",
            Rank::Ipv6 => "x::x",
            Rank::Ipv4 => "x.x.x.x",
            Rank::UserDir => "%USERPROFILE%",
            Rank::Name => "[name]",
        }
    }
}

/// One rule's match, as byte offsets into the original text.
struct Span {
    start: usize,
    end: usize,
    rank: Rank,
}

impl Span {
    fn new(start: usize, end: usize, rank: Rank) -> Self {
        Self { start, end, rank }
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
    let mut names = state.store.known_names().await.map_err(CoreError::from)?;
    // This PC's own names are as private as any device's.
    for var in ["COMPUTERNAME", "USERNAME"] {
        names.extend(std::env::var(var).ok());
    }
    let redactor = Redactor::new(names, user_dir()).map_err(|e| {
        tracing::warn!(error = %e.0, "could not prepare the debug info redaction");
        ApiError::debug_info_unavailable()
    })?;
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
        )
        .unwrap();
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
        let redactor = Redactor::new(vec!["TV".into(), "Lounge TV".into()], None).unwrap();
        assert_eq!(
            redactor.apply("the Lounge TV and the tv"),
            "the [name] and the [name]"
        );
    }

    #[test]
    fn short_names_are_not_redacted() {
        let redactor = Redactor::new(vec!["a".into(), " ".into(), "".into()], None).unwrap();
        assert_eq!(redactor.apply("a lazy cat"), "a lazy cat");
    }

    #[test]
    fn public_addresses_mac_forms_and_clock_times() {
        let redactor = Redactor::new(vec![], None).unwrap();
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
        let redactor = Redactor::new(vec![], Some(r"C:\Users\alice".into())).unwrap();
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

    // ------------------------------------------------ fail-closed names

    #[test]
    fn many_and_very_long_names_are_all_redacted() {
        let mut names: Vec<String> = (0..1500)
            .map(|n| format!("Gadget Ünïcode {n:04}"))
            .collect();
        let long = format!("Friendly {}end", "very long name ".repeat(400));
        names.push(long.clone());
        let redactor = Redactor::new(names.clone(), None).unwrap();
        let out = redactor.apply(&names.join("\n").to_uppercase());
        assert!(!out.contains("GADGET"), "a name leaked");
        // The long one is exact text, so check it in its own case.
        let out = redactor.apply(&format!("see {long} done"));
        assert_eq!(out, "see [name] done");
    }

    #[test]
    fn names_are_replaced_before_addresses_cut_them_up() {
        // The old order (MAC and IP rules first) left the model prefix of
        // these names behind.
        let r = Redactor::new(
            vec![
                "shellyplus1pm-a8032ab12345".into(),
                "cam-192.168.1.20-front".into(),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            r.apply("host shellyplus1pm-a8032ab12345 and CAM-192.168.1.20-FRONT up"),
            "host [name] and [name] up"
        );
    }

    #[test]
    fn hex_like_names_never_cut_an_address_open() {
        for name in ["ab", "10", "bed", "Cafe"] {
            let r = Redactor::new(vec![name.into()], None).unwrap();
            assert_eq!(
                r.apply("aa:bb:cc:ab:ee:ff and 192.168.10.5 and a1bed2c3d4e5 and fe80::ab:1"),
                "xx:xx:xx:xx:xx:xx and x.x.x.x and xx:xx:xx:xx:xx:xx and x::x",
                "name {name}"
            );
            assert_eq!(
                r.apply(&format!("my {name} device")),
                "my [name] device",
                "name {name}"
            );
        }
    }

    #[test]
    fn no_short_name_leaves_address_digits_behind() {
        let addresses = [
            "aa:bb:cc:ab:ee:ff",
            "aa-bb-cc-dd-ee-ff",
            "aabb.ccdd.eeff",
            "a1bed2c3d4e5",
            "192.168.10.5",
            "10.0.0.10",
            "172.16.254.1",
            "fe80::ab:1",
            "2001:db8::10:bed",
        ];
        let names = [
            "ab", "10", "bed", "Cafe", "Dad", "Ace", "AB", "00", "ff", "1", "a1", "be", "1.", "c3",
            "d4e5", "168", "fe80", "db8",
        ];
        for name in names {
            let Ok(r) = Redactor::new(vec![name.into()], None) else {
                panic!("{name}");
            };
            for addr in addresses {
                let out = r.apply(&format!("host {addr} up"));
                for group in addr.split([':', '.', '-']).filter(|g| g.len() > 2) {
                    assert!(
                        !out.contains(group),
                        "name {name} {addr} leaks {group}: {out}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_name_overlapping_an_address_edge_hides_both() {
        let r = Redactor::new(vec!["20 Hub".into()], None).unwrap();
        let out = r.apply("seen 192.168.1.20 Hub");
        for leaked in ["192.168", ".20", "Hub"] {
            assert!(!out.contains(leaked), "{leaked} leaked: {out}");
        }
    }

    #[test]
    fn a_name_reaching_past_an_address_gets_the_address_placeholder() {
        let r = Redactor::new(vec!["20 Hub".into()], None).unwrap();
        assert_eq!(r.apply("seen 192.168.1.20 Hub now"), "seen x.x.x.x now");
    }

    #[test]
    fn a_name_glued_before_a_mac_still_hides_the_mac() {
        let r = Redactor::new(vec!["Plug1".into()], None).unwrap();
        let out = r.apply("seen Plug1aa:bb:cc:dd:ee:ff up");
        assert!(!out.contains("aa:bb:cc:dd:ee:ff"), "{out}");
        assert!(!out.contains("Plug1"), "{out}");
    }

    #[test]
    fn a_name_glued_before_an_ipv4_still_hides_the_ipv4() {
        let r = Redactor::new(vec!["Cam9".into()], None).unwrap();
        let out = r.apply("seen Cam999.0.0.1 up");
        assert!(!out.contains("99.0.0.1"), "{out}");
        assert!(!out.contains("Cam9"), "{out}");
    }

    #[test]
    fn a_multi_byte_name_glued_to_addresses_hides_them() {
        let r = Redactor::new(vec!["Café9".into()], None).unwrap();
        let out = r.apply("seen Café999.0.0.1 and Café9aa:bb:cc:dd:ee:ff ok");
        for leaked in ["99.0.0.1", "aa:bb:cc:dd:ee:ff", "Caf"] {
            assert!(!out.contains(leaked), "{leaked} leaked: {out}");
        }
    }

    #[test]
    fn a_hex_like_name_inside_a_mac_hides_the_whole_mac() {
        let r = Redactor::new(vec!["aa:bb".into()], None).unwrap();
        let out = r.apply("mac 11:22:aa:bb:33:44");
        for leaked in ["11:22", "33:44", "aa:bb"] {
            assert!(!out.contains(leaked), "{leaked} leaked: {out}");
        }
    }

    #[test]
    fn placeholders_survive_a_second_pass_without_leaking() {
        for name in ["na", "me", "xx", "x.", "red", "user"] {
            let r = Redactor::new(vec![name.into()], Some(r"C:\Users\alice".into())).unwrap();
            let out = r.apply(
                r"aa:bb:cc:dd:ee:ff 192.168.1.5 fe80::1 C:\Users\alice\x ?t=0123456789abcdef0123",
            );
            for leaked in ["aa", "192", "168", "fe80", "alice", "0123456789"] {
                assert!(!out.contains(leaked), "name {name} leaked {leaked}: {out}");
            }
        }
    }

    #[test]
    fn a_long_name_is_redacted_in_any_case() {
        let long = format!("Friendly {}end", "very long name ".repeat(400));
        let r = Redactor::new(vec![long.clone()], None).unwrap();
        assert_eq!(
            r.apply(&format!("see {} done", long.to_uppercase())),
            "see [name] done"
        );
        assert_eq!(
            r.apply(&format!("see {} done", long.to_lowercase())),
            "see [name] done"
        );
    }

    #[test]
    fn a_name_that_cannot_compile_is_an_error_never_a_skipped_name() {
        // A size limit this small fits no pattern, so one name alone fails.
        let result = Redactor::with_size_limit(vec!["Lounge TV".into()], None, 1);
        assert!(result.is_err());
        let result = Redactor::with_size_limit(vec![], Some(r"C:\Users\alice".into()), 1);
        assert!(result.is_err());
    }

    #[test]
    fn a_chunk_that_does_not_fit_is_halved_not_dropped() {
        let names: Vec<String> = (0..64).map(|n| format!("device number {n:03}")).collect();
        // Find a limit where the whole chunk fails but halves fit.
        let mut limit = 1 << 10;
        let redactor = loop {
            match Redactor::with_size_limit(names.clone(), None, limit) {
                Ok(r) if r.names.len() > 1 => break r,
                Ok(_) => limit /= 2,
                Err(_) => limit *= 2,
            }
            assert!(limit < 1 << 30 && limit > 0, "no limit splits the chunk");
        };
        let out = redactor.apply(&names.join(" ").to_uppercase());
        assert!(!out.to_lowercase().contains("device"), "{out}");
    }

    // ---------------------------------------------------------- secrets

    #[test]
    fn urls_with_tokens_and_cookies_are_scrubbed() {
        let r = Redactor::new(vec![], None).unwrap();
        let token = "0123456789abcdef0123456789abcdef";
        let out = r.apply(&format!(
            "open http://127.0.0.1:8080/?t={token} then x=1&t={token}z; nl_session=abc123def; \
             Cookie: nl_session=zzz; other=1\nSet-Cookie: a=b\nplain t=short"
        ));
        assert!(!out.contains(token), "{out}");
        assert!(!out.contains("abc123def") && !out.contains("zzz"), "{out}");
        assert!(!out.contains("a=b"), "{out}");
        assert!(out.contains("[redacted]"), "{out}");
        assert!(out.contains("plain t=short"), "{out}");
    }

    // ------------------------------------------ own names and short paths

    #[test]
    fn other_profile_folders_and_the_short_form_are_redacted() {
        let r = Redactor::new(vec![], Some(r"C:\Users\Alice Wonder".into())).unwrap();
        let out = r.apply(
            r#"C:\Users\ALICEW~1\AppData C:\Users\bob\x "C:\\Users\\carol\\y" C:\Users\Public\z C:\Users\Default\q C:\Users\Alice Wonder\w"#,
        );
        assert_eq!(
            out,
            r#"%USERPROFILE%\AppData %USERPROFILE%\x "%USERPROFILE%\\y" C:\Users\Public\z C:\Users\Default\q %USERPROFILE%\w"#
        );
    }

    // -------------------------------------------------------------- MACs

    #[test]
    fn mac_addresses_in_every_form_are_redacted() {
        let r = Redactor::new(vec![], None).unwrap();
        let out = r.apply(
            "a aa:bb-cc:dd-ee:ff b aabb.ccdd.eeff c AABBCCDDEEFF d 0123456789abcdef e 1234567890ab",
        );
        assert_eq!(
            out,
            "a xx:xx:xx:xx:xx:xx b xx:xx:xx:xx:xx:xx c xx:xx:xx:xx:xx:xx d 0123456789abcdef e xx:xx:xx:xx:xx:xx"
        );
    }

    // -------------------------------------------------------------- IPv4

    #[test]
    fn ipv4_next_to_letters_or_underscores_is_redacted_but_versions_are_not() {
        let r = Redactor::new(vec![], None).unwrap();
        let out = r.apply(
            "ip_192.168.1.5 host192.168.1.6 (10.0.0.7) at=10.0.0.8, 1.1.1.1 8.8.8.8 \
             Windows 10.0.26200.1 v1.2.3 1.2.3.4567 999.1.1.1 \n192.168.0.9",
        );
        assert_eq!(
            out,
            "ip_x.x.x.x hostx.x.x.x (x.x.x.x) at=x.x.x.x, 1.1.1.1 8.8.8.8 \
             Windows 10.0.26200.1 v1.2.3 1.2.3.4567 999.1.1.1 \nx.x.x.x"
        );
    }
}
