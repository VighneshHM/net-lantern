//! Developer tasks. `cargo run -p xtask -- oui <oui.csv>` regenerates the vendor table.

use std::path::{Path, PathBuf};

/// Case-insensitive prefix -> fixed short name. First match wins.
const ALIASES: &[(&str, &str)] = &[
    ("TP-LINK", "TP-Link"),
    ("SAMSUNG ELECTRONICS", "Samsung"),
    ("SONY INTERACTIVE", "Sony Interactive"),
    ("AMAZON TECHNOLOGIES", "Amazon"),
    ("SIGNIFY", "Signify (Philips Hue)"),
    ("PHILIPS LIGHTING", "Signify (Philips Hue)"),
    ("LCFC", "Lenovo"),
    ("BEIJING XIAOMI", "Xiaomi"),
    ("XIAOMI", "Xiaomi"),
    ("HEWLETT PACKARD", "HP"),
    ("INTEL CORPORATE", "Intel"),
    ("RING LLC", "Ring"),
    ("HON HAI", "Foxconn"),
    ("GOOGLE", "Google"),
    ("APPLE", "Apple"),
    ("ESPRESSIF", "Espressif"),
    ("SYNOLOGY", "Synology"),
    // Names the classifier matches on, whose IEEE registrations do not start
    // with the brand (or carry a legal-name prefix).
    ("QNAP", "QNAP"),
    ("HANGZHOU HIKVISION", "Hikvision"),
    ("PRAMA HIKVISION", "Hikvision"),
    ("ZHEJIANG DAHUA", "Dahua"),
    ("REOLINK", "Reolink"),
    ("WYZE", "Wyze"),
    ("TUYA", "Tuya"),
    ("SHELLY", "Shelly"),
    ("WIZ", "Wiz"),
    ("BROTHER INDUSTRIES", "Brother"),
    ("LEXMARK", "Lexmark"),
    ("REALTEK", "Realtek"),
];

/// Corporate suffixes stripped from the end of a name, as whole words.
const SUFFIXES: &[&str] = &[
    "Inc",
    "Inc.",
    "Incorporated",
    "Co.",
    "Co.,Ltd",
    "Co., Ltd.",
    "Ltd",
    "Ltd.",
    "LLC",
    "Corporation",
    "Corp.",
    "GmbH",
    "AG",
    "S.A.",
    "B.V.",
    "Limited",
    "Technologies",
    "Technology",
    "Electronics",
];

fn trim_tail(s: &str) -> &str {
    s.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '-'))
}

/// Shorten an IEEE organisation name to the brand-level name used for display.
pub fn short_vendor(org: &str) -> String {
    let org = org.trim();
    let upper = org.to_uppercase();
    for (prefix, name) in ALIASES {
        // The prefix must end on a word boundary, so `APPLE` does not claim
        // "Applied Signal" and `WIZ` does not claim "Wiznet".
        if let Some(rest) = upper.strip_prefix(prefix) {
            if !rest.chars().next().is_some_and(char::is_alphanumeric) {
                return (*name).to_string();
            }
        }
    }

    let mut s = trim_tail(org);
    loop {
        let lower = s.to_lowercase();
        let mut stripped = false;
        for suffix in SUFFIXES {
            let suf = suffix.trim_end_matches('.').to_lowercase();
            if let Some(head) = lower.strip_suffix(&suf) {
                let boundary = head
                    .chars()
                    .next_back()
                    .is_some_and(|c| !c.is_alphanumeric());
                let rest = trim_tail(&s[..head.len()]);
                if boundary && !rest.is_empty() {
                    s = rest;
                    stripped = true;
                    break;
                }
            }
        }
        if !stripped {
            break;
        }
    }

    if s.chars().count() > 4 && s == s.to_uppercase() {
        title_case(s)
    } else {
        s.to_string()
    }
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut at_word_start = true;
    for c in s.chars() {
        if c.is_alphabetic() {
            if at_word_start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            at_word_start = false;
        } else {
            out.push(c);
            at_word_start = !c.is_alphanumeric() && c != '\'';
        }
    }
    out
}

/// True for a block the IEEE sub-assigns through MA-M or MA-S: its MA-L owner
/// says nothing about who made the device, so it gets no vendor.
pub fn is_registration_authority(org: &str) -> bool {
    org.trim()
        .eq_ignore_ascii_case("IEEE Registration Authority")
}

/// Convert the IEEE MA-L CSV text into the sorted TSV table text. Blocks
/// sub-assigned by the IEEE Registration Authority are left out.
pub fn build_table(csv_text: &str) -> Result<String, String> {
    let mut rdr = csv::ReaderBuilder::new().from_reader(csv_text.as_bytes());
    let mut rows: Vec<(String, String)> = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        let (Some(prefix), Some(org)) = (rec.get(1), rec.get(2)) else {
            return Err("row with fewer than 3 columns".into());
        };
        let prefix = prefix.trim().to_uppercase();
        if prefix.len() != 6 || !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("bad assignment `{prefix}`"));
        }
        if is_registration_authority(org) {
            continue;
        }
        let name = short_vendor(org).replace(['\t', '\n', '\r'], " ");
        rows.push((prefix, name));
    }
    rows.sort();
    rows.dedup_by(|a, b| a.0 == b.0);
    let mut out = String::new();
    for (p, n) in rows {
        out.push_str(&p);
        out.push('\t');
        out.push_str(&n);
        out.push('\n');
    }
    Ok(out)
}

fn out_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../netlantern-core/data/oui.tsv")
}

const DESKTOP_CARGO: &str = "apps/desktop/src-tauri/Cargo.toml";
const TAURI_CONF: &str = "apps/desktop/src-tauri/tauri.conf.json";

fn read_file(root: &Path, rel: &str) -> Result<String, String> {
    std::fs::read_to_string(root.join(rel)).map_err(|e| format!("cannot read {rel}: {e}"))
}

fn toml_version(root: &Path, rel: &str, table: &str) -> Result<String, String> {
    let doc: toml::Table = read_file(root, rel)?
        .parse()
        .map_err(|e| format!("{rel} is not valid TOML: {e}"))?;
    let mut node = toml::Value::Table(doc);
    for key in table.split('.') {
        node = node
            .get(key)
            .cloned()
            .ok_or_else(|| format!("{rel} has no [{table}] version"))?;
    }
    let table_value = Some(&node);
    table_value
        .and_then(|t| t.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{rel} has no [{table}] version"))
}

fn workspace_version(root: &Path) -> Result<String, String> {
    toml_version(root, "Cargo.toml", "workspace.package")
}

fn check_version(root: &Path, tag: &str) -> Result<String, String> {
    let version = workspace_version(root)?;
    let desktop = toml_version(root, DESKTOP_CARGO, "package")?;
    if desktop != version {
        return Err(format!(
            "{DESKTOP_CARGO} has version {desktop} but Cargo.toml has {version}"
        ));
    }
    let conf: serde_json::Value = serde_json::from_str(&read_file(root, TAURI_CONF)?)
        .map_err(|e| format!("{TAURI_CONF} is not valid JSON: {e}"))?;
    if let Some(v) = conf.get("version") {
        return Err(format!(
            "{TAURI_CONF} must not set \"version\" (found {v}); Tauri takes it from {DESKTOP_CARGO}, which is {version}"
        ));
    }
    let expected = format!("v{version}");
    if tag != expected {
        return Err(format!(
            "the tag is {tag} but Cargo.toml version {version} needs the tag {expected}"
        ));
    }
    let heading = format!("## {version}");
    if !read_file(root, "CHANGELOG.md")?
        .lines()
        .any(|l| l.trim_end() == heading)
    {
        return Err(format!("CHANGELOG.md has no line \"{heading}\""));
    }
    Ok(version)
}

fn release_notes(changelog: &str, version: &str) -> Option<String> {
    let heading = format!("## {version}");
    let mut lines = changelog.lines();
    lines.find(|l| l.trim_end() == heading)?;
    let body: Vec<&str> = lines.take_while(|l| !l.starts_with("## ")).collect();
    let text = body.join("\n").trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, csv_path] if cmd == "oui" => {
            let text = std::fs::read_to_string(csv_path).unwrap_or_else(|e| {
                eprintln!("cannot read {csv_path}: {e}");
                std::process::exit(1);
            });
            let table = build_table(&text).unwrap_or_else(|e| {
                eprintln!("bad CSV: {e}");
                std::process::exit(1);
            });
            let path = out_path();
            std::fs::write(&path, &table).unwrap_or_else(|e| {
                eprintln!("cannot write {}: {e}", path.display());
                std::process::exit(1);
            });
            println!("wrote {} rows to {}", table.lines().count(), path.display());
        }
        [cmd] if cmd == "version" => match workspace_version(&repo_root()) {
            Ok(v) => println!("{v}"),
            Err(e) => fail(&e),
        },
        [cmd, tag] if cmd == "check-version" => match check_version(&repo_root(), tag) {
            Ok(v) => println!("{v}"),
            Err(e) => fail(&e),
        },
        [cmd, version] if cmd == "release-notes" => {
            let log = read_file(&repo_root(), "CHANGELOG.md").unwrap_or_else(|e| fail(&e));
            match release_notes(&log, version) {
                Some(notes) => println!("{notes}"),
                None => fail(&format!("CHANGELOG.md has no notes for {version}")),
            }
        }
        _ => {
            eprintln!(
                "usage: cargo run -p xtask -- oui <path/to/oui.csv> | version | check-version <tag> | release-notes <version>"
            );
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortens_names() {
        assert_eq!(short_vendor("TP-LINK TECHNOLOGIES CO.,LTD."), "TP-Link");
        assert_eq!(short_vendor("Synology Incorporated"), "Synology");
        assert_eq!(short_vendor("Dell Inc."), "Dell");
        assert_eq!(short_vendor("Espressif Inc."), "Espressif");
        assert_eq!(
            short_vendor("Raspberry Pi Trading Ltd"),
            "Raspberry Pi Trading"
        );
        assert_eq!(short_vendor("AzureWave Technology Inc."), "AzureWave");
    }

    #[test]
    fn strips_repeatedly_and_title_cases() {
        assert_eq!(short_vendor("ACME WIDGETS CO., LTD."), "Acme Widgets");
        assert_eq!(short_vendor("Foo S.A."), "Foo");
        assert_eq!(short_vendor("Incognito Labs"), "Incognito Labs");
        assert_eq!(short_vendor("IBM"), "IBM");
    }

    #[test]
    fn aliases_match_whole_words() {
        assert_eq!(short_vendor("WiZ"), "Wiz");
        assert_eq!(short_vendor("WiZ IoT Company Limited"), "Wiz");
        assert_eq!(short_vendor("Wiznet"), "Wiznet");
        assert_eq!(short_vendor("Apple, Inc."), "Apple");
        assert_eq!(short_vendor("Applied Signal Corp"), "Applied Signal");
        assert_eq!(
            short_vendor("Hangzhou Hikvision Digital Technology Co.,Ltd."),
            "Hikvision"
        );
    }

    #[test]
    fn builds_sorted_table() {
        let csv = "Registry,Assignment,Organization Name,Organization Address\n\
                   MA-L,BB0000,\"Zed, Inc.\",x\nMA-L,AA0000,Dell Inc.,y\n";
        assert_eq!(build_table(csv).unwrap(), "AA0000\tDell\nBB0000\tZed\n");
    }

    #[test]
    fn skips_registration_authority_blocks() {
        let csv = "Registry,Assignment,Organization Name,Organization Address\n\
                   MA-L,B84C87,IEEE Registration Authority,445 Hoes Lane\n\
                   MA-L,AA0000,Dell Inc.,y\n\
                   MA-L,CC0000, ieee registration authority ,z\n";
        assert_eq!(build_table(csv).unwrap(), "AA0000\tDell\n");
        assert!(is_registration_authority("IEEE Registration Authority"));
        assert!(!is_registration_authority("IEEE"));
    }

    mod tempfile_dir {
        use std::path::{Path, PathBuf};
        pub struct Dir(PathBuf);
        impl Dir {
            pub fn new() -> Dir {
                use std::sync::atomic::{AtomicUsize, Ordering};
                static N: AtomicUsize = AtomicUsize::new(0);
                let p = std::env::temp_dir().join(format!(
                    "xtask-test-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::SeqCst)
                ));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
            pub fn write(&self, rel: &str, text: &str) {
                let f = self.0.join(rel);
                std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                std::fs::write(f, text).unwrap();
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn fixture(ws: &str, desktop: &str, tauri: &str, changelog: &str) -> tempfile_dir::Dir {
        let d = tempfile_dir::Dir::new();
        d.write(
            "Cargo.toml",
            &format!("[workspace.package]\nversion = \"{ws}\"\n"),
        );
        d.write(
            "apps/desktop/src-tauri/Cargo.toml",
            &format!("[package]\nname = \"d\"\nversion = \"{desktop}\"\n"),
        );
        d.write("apps/desktop/src-tauri/tauri.conf.json", tauri);
        d.write("CHANGELOG.md", changelog);
        d
    }

    const TAURI_OK: &str = r#"{"productName":"Net Lantern","bundle":{"active":false}}"#;
    const LOG_OK: &str = "# Changelog\n\n## 0.1.0\n- a\n";

    #[test]
    fn matching_versions_pass() {
        let d = fixture("0.1.0", "0.1.0", TAURI_OK, LOG_OK);
        assert_eq!(check_version(d.path(), "v0.1.0"), Ok("0.1.0".to_string()));
        assert_eq!(workspace_version(d.path()), Ok("0.1.0".to_string()));
    }

    #[test]
    fn a_tag_that_disagrees_fails() {
        let d = fixture("0.1.0", "0.1.0", TAURI_OK, LOG_OK);
        let e = check_version(d.path(), "v0.1.1").unwrap_err();
        assert!(e.contains("v0.1.1") && e.contains("0.1.0"), "{e}");
    }

    #[test]
    fn a_desktop_crate_that_disagrees_fails() {
        let d = fixture("0.1.0", "0.2.0", TAURI_OK, LOG_OK);
        let e = check_version(d.path(), "v0.1.0").unwrap_err();
        assert!(
            e.contains("0.2.0") && e.contains("0.1.0") && e.contains("Cargo.toml"),
            "{e}"
        );
    }

    #[test]
    fn a_version_key_in_tauri_conf_fails() {
        let d = fixture("0.1.0", "0.1.0", r#"{"version":"0.1.0"}"#, LOG_OK);
        let e = check_version(d.path(), "v0.1.0").unwrap_err();
        assert!(e.contains("tauri.conf.json") && e.contains("0.1.0"), "{e}");
    }

    #[test]
    fn a_missing_changelog_section_fails() {
        let d = fixture("0.1.0", "0.1.0", TAURI_OK, "# Changelog\n\n## 0.0.9\n- x\n");
        let e = check_version(d.path(), "v0.1.0").unwrap_err();
        assert!(e.contains("CHANGELOG.md") && e.contains("## 0.1.0"), "{e}");
    }

    #[test]
    fn release_notes_stop_at_the_next_section() {
        let log = "## 0.2.0\nB\n## 0.1.0\nA\n";
        assert_eq!(release_notes(log, "0.2.0"), Some("B".to_string()));
        assert_eq!(release_notes(log, "0.1.0"), Some("A".to_string()));
    }

    #[test]
    fn release_notes_of_an_unknown_version_is_none() {
        assert_eq!(release_notes("## 0.1.0\nA\n", "0.3.0"), None);
        assert_eq!(release_notes("## 0.1.0\n\n## 0.0.9\nA\n", "0.1.0"), None);
    }
}
