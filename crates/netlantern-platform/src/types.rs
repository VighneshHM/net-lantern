use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Number of characters in the textual form `XX:XX:XX:XX:XX:XX`.
const MAC_TEXT_LEN: usize = 17;

/// A 48-bit IEEE 802 MAC address.
///
/// Parses from `:`- or `-`-separated hex in any case, displays as uppercase
/// colon-separated hex, and serializes to/from that same string.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    /// True when the locally-administered bit (`0x02` of the first octet) is set,
    /// i.e. the address was assigned by software rather than burned in by a
    /// vendor. Phones use these as "private" Wi-Fi addresses.
    pub fn is_locally_administered(&self) -> bool {
        self.0[0] & 0x02 != 0
    }
}

/// Why a string could not be parsed as a [`MacAddr`].
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum ParseMacError {
    #[error(
        "invalid MAC address length: expected 17 characters like `50:C7:BF:3A:11:02`, got {0}"
    )]
    InvalidLength(usize),
    #[error(
        "invalid MAC address separator `{found}` at position {pos}: expected `{expected}` \
         (use either `:` or `-` throughout)"
    )]
    InvalidSeparator {
        pos: usize,
        found: char,
        expected: char,
    },
    #[error("invalid hex digit `{found}` at position {pos} in MAC address")]
    InvalidHexDigit { pos: usize, found: char },
}

impl FromStr for MacAddr {
    type Err = ParseMacError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() != MAC_TEXT_LEN {
            return Err(ParseMacError::InvalidLength(chars.len()));
        }

        // The first separator decides which one the whole address must use.
        let separator = match chars[2] {
            sep @ (':' | '-') => sep,
            found => {
                return Err(ParseMacError::InvalidSeparator {
                    pos: 2,
                    found,
                    expected: ':',
                })
            }
        };

        let mut octets = [0u8; 6];
        for (i, octet) in octets.iter_mut().enumerate() {
            let start = i * 3;
            if i > 0 && chars[start - 1] != separator {
                return Err(ParseMacError::InvalidSeparator {
                    pos: start - 1,
                    found: chars[start - 1],
                    expected: separator,
                });
            }
            let hi = hex_digit(chars[start], start)?;
            let lo = hex_digit(chars[start + 1], start + 1)?;
            *octet = (hi << 4) | lo;
        }
        Ok(MacAddr(octets))
    }
}

/// Strict hex-digit decoding (`u8::from_str_radix` would also accept a leading `+`).
fn hex_digit(c: char, pos: usize) -> Result<u8, ParseMacError> {
    c.to_digit(16)
        .map(|d| d as u8)
        .ok_or(ParseMacError::InvalidHexDigit { pos, found: c })
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02X}:{b:02X}:{c:02X}:{d:02X}:{e:02X}:{g:02X}")
    }
}

impl Serialize for MacAddr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for MacAddr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colon_and_dash_forms() {
        let a: MacAddr = "50:c7:bf:3a:11:02".parse().unwrap();
        let b: MacAddr = "50-C7-BF-3A-11-02".parse().unwrap();
        assert_eq!(a, b);
        assert_eq!(a.to_string(), "50:C7:BF:3A:11:02");
    }

    #[test]
    fn rejects_short_or_bad_hex() {
        assert!("50:C7".parse::<MacAddr>().is_err());
        assert!("50:C7:BF:3A:11:ZZ".parse::<MacAddr>().is_err());
    }

    #[test]
    fn rejects_mixed_separators_signs_and_non_ascii() {
        // mixed separators
        assert!("50:C7-BF:3A-11:02".parse::<MacAddr>().is_err());
        // `u8::from_str_radix` would accept the leading `+`; we must not
        assert!("+0:C7:BF:3A:11:02".parse::<MacAddr>().is_err());
        // wrong separator character
        assert!("50.C7.BF.3A.11.02".parse::<MacAddr>().is_err());
        // too long, and surrounding whitespace
        assert!("50:C7:BF:3A:11:02:03".parse::<MacAddr>().is_err());
        assert!(" 50:C7:BF:3A:11:02".parse::<MacAddr>().is_err());
        // multi-byte characters never panic or slip through
        assert!("50:C7:BF:3A:11:0é".parse::<MacAddr>().is_err());
        assert!("".parse::<MacAddr>().is_err());
    }

    #[test]
    fn parse_errors_have_helpful_messages() {
        let err = "50:C7".parse::<MacAddr>().unwrap_err();
        assert_eq!(err, ParseMacError::InvalidLength(5));
        assert!(err.to_string().contains("expected 17 characters"));

        let err = "50:C7:BF:3A:11:ZZ".parse::<MacAddr>().unwrap_err();
        assert_eq!(
            err,
            ParseMacError::InvalidHexDigit {
                pos: 15,
                found: 'Z'
            }
        );
        assert!(err.to_string().contains("`Z`"));
    }

    #[test]
    fn detects_private_mac() {
        assert!("DA:A1:19:5C:2E:01"
            .parse::<MacAddr>()
            .unwrap()
            .is_locally_administered());
        assert!(!"50:C7:BF:3A:11:02"
            .parse::<MacAddr>()
            .unwrap()
            .is_locally_administered());
    }

    #[test]
    fn serializes_as_string() {
        let m: MacAddr = "50:C7:BF:3A:11:02".parse().unwrap();
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"50:C7:BF:3A:11:02\"");
        assert_eq!(
            serde_json::from_str::<MacAddr>("\"50:c7:bf:3a:11:02\"").unwrap(),
            m
        );
    }

    #[test]
    fn deserializing_a_bad_string_fails() {
        assert!(serde_json::from_str::<MacAddr>("\"not-a-mac\"").is_err());
        assert!(serde_json::from_str::<MacAddr>("42").is_err());
    }
}
