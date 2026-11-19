//! The deep scan's port catalogue, banner cleaning and page-title extraction.

/// The deep scan's ports: 100, ascending, a superset of the quick scan's.
pub const DEEP_PORTS: [u16; 100] = [
    21, 22, 23, 25, 53, 80, 81, 88, 110, 111, 135, 139, 143, 389, 427, 443, 445, 465, 515, 548,
    554, 587, 631, 873, 990, 993, 995, 1025, 1080, 1400, 1433, 1723, 1883, 2000, 2049, 2222, 2869,
    3000, 3001, 3128, 3306, 3389, 3689, 4070, 4443, 5000, 5001, 5060, 5222, 5357, 5432, 5555, 5800,
    5900, 5985, 6000, 6379, 6466, 6467, 6668, 7000, 7547, 8000, 8001, 8002, 8008, 8009, 8060, 8080,
    8081, 8086, 8088, 8090, 8123, 8181, 8200, 8291, 8443, 8554, 8880, 8883, 8888, 9000, 9080, 9090,
    9091, 9100, 9197, 9295, 9443, 9999, 10000, 32400, 34567, 37777, 49152, 49153, 50000, 55443,
    62078,
];

/// What the banner stage does with an open port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Http,
    Https,
    Greeting,
    Nothing,
}

pub fn probe_kind(port: u16) -> Probe {
    match port {
        80 | 81 | 1400 | 2869 | 3000 | 3001 | 5000 | 5800 | 7547 | 8000 | 8001 | 8002 | 8008
        | 8060 | 8080 | 8081 | 8086 | 8088 | 8090 | 8123 | 8181 | 8200 | 8880 | 8888 | 9000
        | 9080 | 9090 | 9091 | 9197 | 10000 | 32400 | 49152 | 49153 | 50000 => Probe::Http,
        443 | 4443 | 5001 | 8443 | 9443 => Probe::Https,
        21 | 22 | 23 | 25 | 110 | 143 | 587 | 2222 | 5900 => Probe::Greeting,
        _ => Probe::Nothing,
    }
}

/// The IANA service name, or the product commonly behind the port.
pub fn service_name(port: u16) -> Option<&'static str> {
    Some(match port {
        21 => "ftp",
        22 => "ssh",
        23 => "telnet",
        25 => "smtp",
        53 => "dns",
        80 => "http",
        81 => "http-alt",
        88 => "kerberos",
        110 => "pop3",
        111 => "rpcbind",
        135 => "msrpc",
        139 => "netbios-ssn",
        143 => "imap",
        389 => "ldap",
        427 => "slp",
        443 => "https",
        445 => "smb",
        465 => "smtps",
        515 => "lpd",
        548 => "afp",
        554 => "rtsp",
        587 => "submission",
        631 => "ipp",
        873 => "rsync",
        990 => "ftps",
        993 => "imaps",
        995 => "pop3s",
        1025 => "windows-rpc",
        1080 => "socks",
        1400 => "sonos",
        1433 => "mssql",
        1723 => "pptp",
        1883 => "mqtt",
        2000 => "mikrotik-bandwidth",
        2049 => "nfs",
        2222 => "ssh-alt",
        2869 => "upnp",
        3000 => "web-app",
        3001 => "web-app",
        3128 => "proxy",
        3306 => "mysql",
        3389 => "rdp",
        3689 => "daap",
        4070 => "spotify-connect",
        4443 => "https-alt",
        5000 => "upnp-web",
        5001 => "synology-https",
        5060 => "sip",
        5222 => "xmpp",
        5357 => "wsdapi",
        5432 => "postgresql",
        5555 => "adb",
        5800 => "vnc-http",
        5900 => "vnc",
        5985 => "winrm",
        6000 => "x11",
        6379 => "redis",
        6466 => "androidtv-remote",
        6467 => "androidtv-remote",
        6668 => "tuya",
        7000 => "airplay",
        7547 => "tr-069",
        8000 => "http-alt",
        8001 => "http-alt",
        8002 => "http-alt",
        8008 => "chromecast-http",
        8009 => "chromecast",
        8060 => "roku",
        8080 => "http-proxy",
        8081 => "http-alt",
        8086 => "influxdb",
        8088 => "http-alt",
        8090 => "http-alt",
        8123 => "home-assistant",
        8181 => "http-alt",
        8200 => "dlna",
        8291 => "winbox",
        8443 => "https-alt",
        8554 => "rtsp-alt",
        8880 => "http-alt",
        8883 => "mqtt-tls",
        8888 => "http-alt",
        9000 => "web-app",
        9080 => "http-alt",
        9090 => "web-admin",
        9091 => "transmission",
        9100 => "printer",
        9197 => "upnp-alt",
        9295 => "ps-remote-play",
        9443 => "https-alt",
        9999 => "web-app",
        10000 => "webmin",
        32400 => "plex",
        34567 => "dvr",
        37777 => "dahua-dvr",
        49152 => "upnp-dynamic",
        49153 => "upnp-dynamic",
        50000 => "upnp-dynamic",
        55443 => "smart-device",
        62078 => "iphone-sync",
        _ => return None,
    })
}

const BANNER_MAX_CHARS: usize = 100;

/// Bidirectional and invisible formatting characters, which could make a
/// banner read differently from what it says.
fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// First non-empty line, control and bidi/format characters removed,
/// trimmed, cut to 100 chars.
pub fn clean_banner(raw: &str) -> Option<String> {
    raw.split('\n').find_map(|line| {
        let cleaned: String = line
            .chars()
            .filter(|c| !c.is_control() && !is_format(*c))
            .collect();
        let trimmed = cleaned.trim();
        if trimmed.is_empty() {
            return None;
        }
        let cut: String = trimmed.chars().take(BANNER_MAX_CHARS).collect();
        Some(cut.trim_end().to_string())
    })
}

/// The `<title>` (case-insensitive), with &amp; &lt; &gt; &quot; &#39; decoded,
/// whitespace collapsed, then `clean_banner`.
pub fn html_title(body: &str) -> Option<String> {
    let lower = body.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    let collapsed = body[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let decoded = collapsed
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    clean_banner(&decoded)
}

/// The page title, else `"Server: <header>"`, else None.
pub fn http_banner(resp: &netlantern_platform::HttpResponse) -> Option<String> {
    html_title(&resp.body_prefix).or_else(|| {
        let server = clean_banner(resp.server.as_deref()?)?;
        clean_banner(&format!("Server: {server}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_ports_are_100_sorted_unique_and_cover_the_quick_list() {
        assert_eq!(DEEP_PORTS.len(), 100);
        assert!(DEEP_PORTS.windows(2).all(|w| w[0] < w[1]));
        for p in [80, 443, 445, 22, 62078] {
            assert!(DEEP_PORTS.contains(&p), "{p}");
        }
        for p in DEEP_PORTS {
            assert!(service_name(p).is_some(), "no name for {p}");
        }
        assert_eq!(service_name(12345), None);
        assert_eq!(service_name(62078), Some("iphone-sync"));
    }

    #[test]
    fn probe_kinds() {
        assert_eq!(probe_kind(8080), Probe::Http);
        assert_eq!(probe_kind(8443), Probe::Https);
        assert_eq!(probe_kind(22), Probe::Greeting);
        assert_eq!(probe_kind(445), Probe::Nothing);
        // MySQL's handshake is binary, not a text greeting.
        assert_eq!(probe_kind(3306), Probe::Nothing);
        assert_eq!(service_name(3306), Some("mysql"));
    }

    #[test]
    fn bidi_and_format_characters_are_dropped_but_letters_kept() {
        assert_eq!(
            clean_banner("Router\u{202E}gnp.exe\u{202C} admin").as_deref(),
            Some("Routergnp.exe admin")
        );
        assert_eq!(
            clean_banner("\u{FEFF}\u{200B}a\u{200F}b\u{2060}c\u{2066}d\u{2069}e\u{202A}")
                .as_deref(),
            Some("abcde")
        );
        assert_eq!(
            clean_banner("Caf\u{e9} \u{8def}\u{7531}\u{5668}").as_deref(),
            Some("Caf\u{e9} \u{8def}\u{7531}\u{5668}")
        );
    }

    #[test]
    fn banners_are_cleaned_and_cut() {
        assert_eq!(
            clean_banner("\r\n  SSH-2.0-OpenSSH_9.6\r\nrest").as_deref(),
            Some("SSH-2.0-OpenSSH_9.6")
        );
        assert_eq!(
            clean_banner("220 ftp\u{0007}\u{001b}[31m ready").as_deref(),
            Some("220 ftp[31m ready")
        );
        assert_eq!(
            clean_banner(&"x".repeat(5000)).map(|s| s.chars().count()),
            Some(100)
        );
        assert_eq!(clean_banner(" \r\n\t"), None);
        assert_eq!(
            clean_banner("caf\u{FFFD} box").as_deref(),
            Some("caf\u{FFFD} box")
        );
    }

    #[test]
    fn titles_are_decoded_and_never_markup() {
        assert_eq!(
            html_title("<HTML><TiTlE>\n  Router &amp; Modem\n</title>").as_deref(),
            Some("Router & Modem")
        );
        assert_eq!(
            html_title("<title><script>alert(1)</script></title>").as_deref(),
            Some("<script>alert(1)</script>")
        );
        assert_eq!(html_title("<title></title>"), None);
        assert_eq!(html_title("no title here"), None);
    }

    #[test]
    fn http_banner_prefers_the_title_then_the_server_header() {
        let r = |server: Option<&str>, body: &str| netlantern_platform::HttpResponse {
            status: 200,
            location: None,
            server: server.map(String::from),
            body_prefix: body.into(),
        };
        assert_eq!(
            http_banner(&r(Some("lighttpd"), "<title>Hub</title>")).as_deref(),
            Some("Hub")
        );
        assert_eq!(
            http_banner(&r(Some("lighttpd"), "")).as_deref(),
            Some("Server: lighttpd")
        );
        assert_eq!(http_banner(&r(None, "")), None);
    }
}
