//! Rule-table device type classifier.

use netlantern_platform::{MdnsService, NetbiosInfo, SsdpReply};

use super::upnp::UpnpDescription;
use crate::model::DeviceType;

/// Everything learned about one device.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Signals {
    pub vendor: Option<String>,
    pub mdns: Vec<MdnsService>,
    pub ssdp: Vec<(SsdpReply, Option<UpnpDescription>)>,
    pub netbios: Option<NetbiosInfo>,
    pub rdns: Option<String>,
    pub open_ports: Vec<u16>,
    pub ttl: Option<u8>,
    pub private_mac: bool,
    pub is_gateway: bool,
    pub is_self: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Classification {
    pub device_type: DeviceType,
    pub confidence: f32,
}

/// What a rule looks for. Every text match is case-insensitive.
enum Matcher {
    IsSelf,
    IsGateway,
    /// Substring of an SSDP `ST` or of a fetched UPnP `deviceType`.
    Ssdp(&'static [&'static str]),
    /// Substring of an SSDP `SERVER` header.
    SsdpServer(&'static [&'static str]),
    /// Whole word in a fetched UPnP friendly name.
    UpnpNameWord(&'static str),
    /// mDNS service type (without `.local`).
    Mdns(&'static [&'static str]),
    /// Substring of an mDNS model.
    MdnsModel(&'static [&'static str]),
    Port(u16),
    /// Substring of the announced hostname.
    Host(&'static [&'static str]),
    /// Whole word in the announced hostname.
    HostWord(&'static str),
    /// `hp` followed by six hex digits in the announced hostname.
    HostHp,
    /// Exact vendor short name.
    Vendor(&'static [&'static str]),
    PrivateMac,
    Ttl(u8),
    Any(&'static [Matcher]),
}

use DeviceType::*;
use Matcher::*;

/// The rule table. Earlier rows win ties. New rows are data, not code.
const RULES: &[(Matcher, &[(DeviceType, u8)])] = &[
    (IsSelf, &[(Laptop, 10)]),
    (IsGateway, &[(Router, 10)]),
    (Ssdp(&["InternetGatewayDevice"]), &[(Router, 8)]),
    (
        Any(&[
            Mdns(&[
                "_ipp._tcp",
                "_ipps._tcp",
                "_printer._tcp",
                "_pdl-datastream._tcp",
            ]),
            Ssdp(&["Printer"]),
        ]),
        &[(Printer, 8)],
    ),
    (
        Any(&[Mdns(&["_sonos._tcp"]), Ssdp(&["ZonePlayer"])]),
        &[(Speaker, 8)],
    ),
    (
        MdnsModel(&[
            "nest mini",
            "nest audio",
            "home mini",
            "nest hub",
            "homepod",
            "echo",
        ]),
        &[(Speaker, 8)],
    ),
    (Mdns(&["_googlecast._tcp"]), &[(Tv, 5)]),
    (
        Any(&[
            Mdns(&["_androidtvremote2._tcp", "_amzn-wplay._tcp"]),
            Ssdp(&["dial-multiscreen"]),
            SsdpServer(&["Roku"]),
        ]),
        &[(Tv, 7)],
    ),
    (
        Any(&[Ssdp(&["MediaRenderer"]), UpnpNameWord("TV")]),
        &[(Tv, 5)],
    ),
    (
        Mdns(&["_raop._tcp", "_spotify-connect._tcp"]),
        &[(Speaker, 4)],
    ),
    (Mdns(&["_airplay._tcp"]), &[(Tv, 3)]),
    (
        Any(&[
            Mdns(&["_hap._tcp", "_hue._tcp", "_matter._tcp"]),
            SsdpServer(&["IpBridge"]),
        ]),
        &[(Iot, 6)],
    ),
    (Mdns(&["_afpovertcp._tcp"]), &[(Nas, 4)]),
    (Mdns(&["_smb._tcp"]), &[(Nas, 3), (Laptop, 2)]),
    (
        Any(&[Mdns(&["_apple-mobdev2._tcp"]), Port(62078)]),
        &[(Phone, 5), (Tablet, 3)],
    ),
    (
        Mdns(&["_companion-link._tcp"]),
        &[(Phone, 2), (Tablet, 2), (Laptop, 1)],
    ),
    (Mdns(&["_workstation._tcp", "_rdp._tcp"]), &[(Laptop, 5)]),
    (Port(445), &[(Laptop, 3), (Nas, 2)]),
    (Port(22), &[(Nas, 1), (Laptop, 1)]),
    (
        Host(&["iphone", "android", "pixel", "galaxy", "oneplus", "redmi"]),
        &[(Phone, 8)],
    ),
    (Host(&["ipad", "tab-", "galaxy-tab"]), &[(Tablet, 9)]),
    (
        Host(&["macbook", "desktop-", "laptop-", "-pc", "imac", "laptop"]),
        &[(Laptop, 6)],
    ),
    (
        Any(&[Host(&["printer", "epson", "canon", "brother"]), HostHp]),
        &[(Printer, 6)],
    ),
    (
        Any(&[
            Host(&["bravia", "roku", "firetv", "chromecast"]),
            HostWord("tv"),
        ]),
        &[(Tv, 6)],
    ),
    (
        Host(&["echo", "sonos", "homepod", "nest-mini", "speaker"]),
        &[(Speaker, 6)],
    ),
    (Host(&["cam", "camera", "doorbell"]), &[(Camera, 5)]),
    (
        Host(&["nas", "synology", "diskstation", "qnap"]),
        &[(Nas, 7)],
    ),
    (
        Host(&["xbox", "playstation", "ps4", "ps5", "nintendo"]),
        &[(Console, 7)],
    ),
    (
        Host(&["esp_", "esp-", "tasmota", "shelly", "tuya", "wled"]),
        &[(Iot, 6)],
    ),
    (Vendor(&["Synology", "QNAP"]), &[(Nas, 7)]),
    (Vendor(&["Sonos"]), &[(Speaker, 8)]),
    (Vendor(&["Roku"]), &[(Tv, 8)]),
    (
        Vendor(&["Ring", "Hikvision", "Dahua", "Reolink", "Wyze", "Arlo"]),
        &[(Camera, 7)],
    ),
    (Vendor(&["Sony Interactive", "Nintendo"]), &[(Console, 7)]),
    (
        Vendor(&[
            "Espressif",
            "Tuya",
            "Shelly",
            "Signify (Philips Hue)",
            "Itead",
            "Wiz",
        ]),
        &[(Iot, 5)],
    ),
    (
        Vendor(&[
            "Brother",
            "Epson",
            "Seiko Epson",
            "Canon",
            "Lexmark",
            "Xerox",
        ]),
        &[(Printer, 6)],
    ),
    (
        Vendor(&[
            "Intel",
            "Dell",
            "Lenovo",
            "AzureWave",
            "Liteon",
            "Realtek",
            "Raspberry Pi Trading",
        ]),
        &[(Laptop, 4)],
    ),
    (Vendor(&["Amazon"]), &[(Speaker, 3), (Tv, 2)]),
    (Vendor(&["Google"]), &[(Speaker, 2), (Tv, 2), (Phone, 1)]),
    (Vendor(&["Samsung"]), &[(Phone, 2), (Tv, 2)]),
    (Vendor(&["Apple"]), &[(Phone, 2), (Laptop, 1), (Tablet, 1)]),
    (Vendor(&["HP"]), &[(Printer, 3), (Laptop, 2)]),
    (Vendor(&["Microsoft"]), &[(Console, 2), (Laptop, 2)]),
    (PrivateMac, &[(Phone, 2), (Tablet, 1)]),
    (Ttl(128), &[(Laptop, 2)]),
];

/// Types in the order they are listed in [`DeviceType`], for indexing.
const TYPES: [DeviceType; 11] = [
    Router, Laptop, Phone, Tablet, Printer, Tv, Speaker, Camera, Nas, Console, Iot,
];

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn any_contains(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| contains_ci(haystack, n))
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|w| w.eq_ignore_ascii_case(word))
}

/// True when `text` has `hp` immediately followed by six hex digits.
fn has_hp_hex(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.match_indices("hp").any(|(i, _)| {
        let tail = &lower.as_bytes()[i + 2..];
        tail.len() >= 6 && tail[..6].iter().all(u8::is_ascii_hexdigit)
    })
}

impl Matcher {
    fn matches(&self, s: &Signals, hostname: &str) -> bool {
        match self {
            IsSelf => s.is_self,
            IsGateway => s.is_gateway,
            Ssdp(needles) => s.ssdp.iter().any(|(r, d)| {
                any_contains(&r.st, needles)
                    || d.as_ref()
                        .and_then(|d| d.device_type.as_deref())
                        .is_some_and(|t| any_contains(t, needles))
            }),
            SsdpServer(needles) => s.ssdp.iter().any(|(r, _)| {
                r.server
                    .as_deref()
                    .is_some_and(|v| any_contains(v, needles))
            }),
            UpnpNameWord(word) => s.ssdp.iter().any(|(_, d)| {
                d.as_ref()
                    .and_then(|d| d.friendly_name.as_deref())
                    .is_some_and(|n| has_word(n, word))
            }),
            Mdns(types) => s
                .mdns
                .iter()
                .any(|m| types.iter().any(|t| m.service_type.eq_ignore_ascii_case(t))),
            MdnsModel(needles) => s
                .mdns
                .iter()
                .any(|m| m.model.as_deref().is_some_and(|v| any_contains(v, needles))),
            Port(p) => s.open_ports.contains(p),
            Host(needles) => any_contains(hostname, needles),
            HostWord(word) => has_word(hostname, word),
            HostHp => has_hp_hex(hostname),
            Vendor(names) => s
                .vendor
                .as_deref()
                .is_some_and(|v| names.iter().any(|n| v.eq_ignore_ascii_case(n))),
            PrivateMac => s.private_mac,
            Ttl(t) => s.ttl == Some(*t),
            Any(ms) => ms.iter().any(|m| m.matches(s, hostname)),
        }
    }
}

/// Guess a device's type from its evidence. `hostname` is the announced name.
///
/// Every matching rule adds its weights; the highest total wins, ties going
/// to the type whose first contributing rule comes earlier in the table. A
/// top total below 4 is `Unknown`. Confidence is `min(top, 10) / 10`.
pub fn classify(s: &Signals, hostname: Option<&str>) -> Classification {
    let hostname = hostname.unwrap_or("");
    let mut sums = [0u32; TYPES.len()];
    let mut first_row = [usize::MAX; TYPES.len()];
    for (row, (matcher, weights)) in RULES.iter().enumerate() {
        if !matcher.matches(s, hostname) {
            continue;
        }
        for (ty, w) in *weights {
            let i = TYPES.iter().position(|t| t == ty).expect("listed type");
            sums[i] += u32::from(*w);
            first_row[i] = first_row[i].min(row);
        }
    }
    let best = (0..TYPES.len())
        .filter(|&i| sums[i] > 0)
        .min_by_key(|&i| (std::cmp::Reverse(sums[i]), first_row[i]));
    match best {
        Some(i) if sums[i] >= 4 => Classification {
            device_type: TYPES[i],
            confidence: sums[i].min(10) as f32 / 10.0,
        },
        _ => Classification {
            device_type: Unknown,
            confidence: best.map_or(0.0, |i| sums[i].min(10) as f32 / 10.0),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use netlantern_platform::{MacAddr, MdnsService, NetbiosInfo, SsdpReply};

    use super::*;
    use crate::identify::names::announced_name;
    use crate::identify::oui;
    use crate::identify::upnp::parse_upnp_description;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn ip0() -> Ipv4Addr {
        ip("10.255.255.255")
    }

    fn mac(s: &str) -> MacAddr {
        s.parse().unwrap()
    }

    fn mdns(
        a: &str,
        service: &str,
        instance: &str,
        hostname: Option<&str>,
        friendly: Option<&str>,
        model: Option<&str>,
    ) -> MdnsService {
        MdnsService {
            ip: ip(a),
            service_type: service.into(),
            instance: instance.into(),
            hostname: hostname.map(Into::into),
            friendly_name: friendly.map(Into::into),
            model: model.map(Into::into),
        }
    }

    fn ssdp(
        a: &str,
        st: &str,
        server: Option<&str>,
        loc: Option<&str>,
        xml: Option<&str>,
    ) -> (SsdpReply, Option<UpnpDescription>) {
        (
            SsdpReply {
                ip: ip(a),
                st: st.into(),
                server: server.map(Into::into),
                location: loc.map(Into::into),
            },
            xml.map(parse_upnp_description),
        )
    }

    fn base(m: &str, ttl: Option<u8>, ports: &[u16]) -> Signals {
        let mac = mac(m);
        Signals {
            vendor: oui::vendor(mac).map(Into::into),
            open_ports: ports.to_vec(),
            ttl,
            private_mac: mac.is_locally_administered(),
            ..Default::default()
        }
    }

    /// The 13 devices of scenarios/home.json with the evidence discovery
    /// would gather for them.
    fn home_signals() -> Vec<(Signals, DeviceType)> {
        let mut v = Vec::new();
        let mut s = base("50:C7:BF:3A:11:02", Some(64), &[53, 80, 443]);
        s.is_gateway = true;
        s.ssdp = vec![ssdp(
            "192.168.1.1",
            "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
            None,
            Some("http://192.168.1.1:1900/igd.xml"),
            Some("<root><device><friendlyName>Archer AX55</friendlyName><deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType></device></root>"),
        )];
        s.rdns = Some("router.lan".into());
        v.push((s, Router));

        let mut s = base("F4:8E:38:9A:20:7C", Some(128), &[]);
        s.is_self = true;
        s.netbios = Some(NetbiosInfo {
            name: "LANTERN-LAPTOP".into(),
            workgroup: None,
        });
        v.push((s, Laptop));

        let mut s = base("3C:28:6D:91:4E:B0", Some(64), &[]);
        s.mdns = vec![mdns(
            "192.168.1.31",
            "_googlecast._tcp",
            "Google-Nest-Mini-3c1e",
            Some("3c1e9a7b-1f.local"),
            Some("Kitchen speaker"),
            Some("Google Nest Mini"),
        )];
        v.push((s, Speaker));

        let mut s = base("DA:A1:19:5C:2E:01", Some(64), &[]);
        s.rdns = Some("Pixel-7.lan".into());
        v.push((s, Phone));

        let mut s = base("8C:79:F5:0B:6A:31", Some(64), &[8001, 8002]);
        s.ssdp = vec![ssdp(
            "192.168.1.52",
            "urn:schemas-upnp-org:device:MediaRenderer:1",
            Some("Samsung UPnP/1.0"),
            Some("http://192.168.1.52:9197/dmr"),
            Some("<root><device><friendlyName>[TV] Living Room</friendlyName><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType></device></root>"),
        )];
        v.push((s, Tv));

        let mut s = base("F0:18:98:7C:3D:5E", Some(64), &[62078]);
        s.mdns = vec![mdns(
            "192.168.1.57",
            "_apple-mobdev2._tcp",
            "iPhone",
            Some("iPhone.local"),
            None,
            None,
        )];
        v.push((s, Phone));

        let mut s = base("44:65:0D:A2:9B:17", Some(64), &[]);
        s.rdns = Some("echo-dot.lan".into());
        v.push((s, Speaker));

        let mut s = base("00:11:32:AB:CD:70", Some(64), &[22, 445, 5000]);
        s.netbios = Some(NetbiosInfo {
            name: "DISKSTATION".into(),
            workgroup: Some("WORKGROUP".into()),
        });
        s.mdns = vec![
            mdns(
                "192.168.1.70",
                "_smb._tcp",
                "DiskStation",
                Some("DiskStation.local"),
                None,
                None,
            ),
            mdns(
                "192.168.1.70",
                "_afpovertcp._tcp",
                "DiskStation",
                Some("DiskStation.local"),
                None,
                None,
            ),
        ];
        v.push((s, Nas));

        v.push((base("24:6F:28:3A:9F:21", None, &[80]), Iot));
        v.push((base("00:E4:21:6B:8C:40", Some(64), &[]), Console));
        v.push((base("54:E0:19:2D:77:A8", Some(64), &[]), Camera));

        let mut s = base("98:FA:9B:44:02:6D", Some(128), &[445]);
        s.netbios = Some(NetbiosInfo {
            name: "WORK-LAPTOP".into(),
            workgroup: None,
        });
        v.push((s, Laptop));

        let mut s = base("EC:B5:FA:12:88:0C", Some(64), &[80, 443]);
        s.mdns = vec![mdns(
            "192.168.1.130",
            "_hue._tcp",
            "Hue Bridge - 1A2B3C",
            None,
            None,
            None,
        )];
        s.ssdp = vec![ssdp(
            "192.168.1.130",
            "upnp:rootdevice",
            Some("Hue/1.0 UPnP/1.0 IpBridge"),
            None,
            None,
        )];
        v.push((s, Iot));
        v
    }

    #[test]
    fn classifies_home_devices() {
        let all = home_signals();
        assert_eq!(all.len(), 13);
        for (signals, expected) in all {
            let name = announced_name(&signals, ip0());
            let c = classify(&signals, name.as_deref());
            assert_eq!(
                c.device_type, expected,
                "vendor {:?} name {:?}",
                signals.vendor, name
            );
        }
    }

    #[test]
    fn weak_evidence_is_unknown() {
        let s = Signals {
            vendor: Some("Apple".into()),
            ..Default::default()
        };
        assert_eq!(
            classify(&s, None),
            Classification {
                device_type: Unknown,
                confidence: 0.2
            }
        );
    }

    #[test]
    fn gateway_always_router() {
        let s = Signals {
            vendor: Some("Samsung".into()),
            is_gateway: true,
            ..Default::default()
        };
        assert_eq!(
            classify(&s, None),
            Classification {
                device_type: Router,
                confidence: 1.0
            }
        );
    }

    #[test]
    fn ties_go_to_earlier_row() {
        let s = Signals {
            is_self: true,
            is_gateway: true,
            ..Default::default()
        };
        assert_eq!(classify(&s, None).device_type, Laptop);
    }

    #[test]
    fn hostname_rules() {
        let s = Signals::default();
        assert_eq!(classify(&s, Some("Living-Room-TV")).device_type, Tv);
        assert_eq!(classify(&s, Some("outlet")).device_type, Unknown);
        assert_eq!(classify(&s, Some("HP3A4B5C")).device_type, Printer);
        assert_eq!(classify(&s, Some("hp12")).device_type, Unknown);
        assert_eq!(classify(&s, Some("Galaxy-Tab-S8")).device_type, Tablet);
        assert_eq!(classify(&s, Some("DESKTOP-ABC")).device_type, Laptop);
    }

    #[test]
    fn confidence_caps_at_one() {
        let s = Signals {
            is_self: true,
            ttl: Some(128),
            ..Default::default()
        };
        assert_eq!(
            classify(&s, None),
            Classification {
                device_type: Laptop,
                confidence: 1.0
            }
        );
    }
}
