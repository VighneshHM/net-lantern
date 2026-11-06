//! Minimal UPnP device-description parsing (plain string search, no XML crate).

/// The fields of a UPnP description Net Lantern cares about.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UpnpDescription {
    pub friendly_name: Option<String>,
    pub device_type: Option<String>,
    pub manufacturer: Option<String>,
    pub model_name: Option<String>,
}

/// Text of the first element named `name`, ignoring any namespace prefix.
fn first_element_text(xml: &str, name: &str) -> Option<String> {
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        let gt = rest.find('>')?;
        let tag = &rest[..gt];
        rest = &rest[gt + 1..];
        if tag.starts_with(['/', '!', '?']) || tag.ends_with('/') {
            continue;
        }
        let tag_name = tag.split_whitespace().next().unwrap_or("");
        let local = tag_name.rsplit(':').next().unwrap_or("");
        if local == name {
            let end = rest.find('<').unwrap_or(rest.len());
            let text = decode_entities(rest[..end].trim());
            return if text.is_empty() { None } else { Some(text) };
        }
    }
    None
}

fn decode_entities(s: &str) -> String {
    // `&amp;` last so that `&amp;lt;` decodes to `&lt;`, not `<`.
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Pull the identifying fields out of a UPnP device description. Takes the
/// first occurrence of each element; anything missing or malformed is `None`.
pub fn parse_upnp_description(xml: &str) -> UpnpDescription {
    UpnpDescription {
        friendly_name: first_element_text(xml, "friendlyName"),
        device_type: first_element_text(xml, "deviceType"),
        manufacturer: first_element_text(xml, "manufacturer"),
        model_name: first_element_text(xml, "modelName"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upnp_description_fields() {
        let d = parse_upnp_description("<root><device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType><friendlyName> [TV] Living Room </friendlyName><manufacturer>Samsung &amp; Co</manufacturer></device></root>");
        assert_eq!(d.friendly_name.as_deref(), Some("[TV] Living Room"));
        assert_eq!(d.manufacturer.as_deref(), Some("Samsung & Co"));
        assert_eq!(
            d.device_type.as_deref(),
            Some("urn:schemas-upnp-org:device:MediaRenderer:1")
        );
        assert_eq!(d.model_name, None);
    }

    #[test]
    fn ignores_namespace_prefixes_and_takes_first() {
        let d = parse_upnp_description(
            "<u:root><u:device><u:friendlyName>A</u:friendlyName><u:friendlyName>B</u:friendlyName><modelName>X &lt;1&gt; &quot;q&quot; &apos;s&apos;</modelName></u:device></u:root>",
        );
        assert_eq!(d.friendly_name.as_deref(), Some("A"));
        assert_eq!(d.model_name.as_deref(), Some("X <1> \"q\" 's'"));
    }

    #[test]
    fn empty_and_garbage_input() {
        assert_eq!(parse_upnp_description(""), UpnpDescription::default());
        assert_eq!(
            parse_upnp_description("<friendlyName>   </friendlyName><x"),
            UpnpDescription::default()
        );
        assert_eq!(
            parse_upnp_description("<friendlyName"),
            UpnpDescription::default()
        );
    }
}
