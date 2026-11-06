use std::net::Ipv4Addr;

/// Widest network (smallest prefix length) that is swept in full.
pub const MAX_SWEEP_PREFIX: u8 = 22;

/// Host addresses a scan sweeps for `ip` on a network with the given `prefix`.
///
/// Addresses are in ascending order and exclude the network and broadcast
/// addresses. Networks wider than [`MAX_SWEEP_PREFIX`] fall back to the /24
/// containing `ip`; point-to-point prefixes (/31 and longer) sweep only `ip`.
pub fn scan_targets(ip: Ipv4Addr, prefix: u8) -> Vec<Ipv4Addr> {
    if prefix >= 31 {
        return vec![ip];
    }
    let prefix = if prefix < MAX_SWEEP_PREFIX {
        24
    } else {
        prefix
    };
    let mask = u32::MAX << (32 - u32::from(prefix));
    let network = u32::from(ip) & mask;
    let broadcast = network | !mask;
    (network + 1..broadcast).map(Ipv4Addr::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn slash24() {
        let t = scan_targets(ip("192.168.1.23"), 24);
        assert_eq!(
            (t.len(), t[0], *t.last().unwrap()),
            (254, ip("192.168.1.1"), ip("192.168.1.254"))
        );
    }

    #[test]
    fn slash22() {
        let t = scan_targets(ip("10.0.5.9"), 22);
        assert_eq!(
            (t.len(), t[0], *t.last().unwrap()),
            (1022, ip("10.0.4.1"), ip("10.0.7.254"))
        );
    }

    #[test]
    fn larger_than_22_scans_own_24() {
        let t = scan_targets(ip("10.0.5.9"), 16);
        assert_eq!(
            (t.len(), t[0], *t.last().unwrap()),
            (254, ip("10.0.5.1"), ip("10.0.5.254"))
        );
    }

    #[test]
    fn point_to_point() {
        assert_eq!(scan_targets(ip("10.0.0.1"), 32), vec![ip("10.0.0.1")]);
    }

    #[test]
    fn slash30_has_two_hosts() {
        assert_eq!(
            scan_targets(ip("10.0.0.5"), 30),
            vec![ip("10.0.0.5"), ip("10.0.0.6")]
        );
    }

    #[test]
    fn slash31_is_point_to_point() {
        assert_eq!(scan_targets(ip("10.0.0.4"), 31), vec![ip("10.0.0.4")]);
    }
}
