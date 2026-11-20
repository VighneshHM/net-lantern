//! Read-only view of the Windows Firewall: whether each profile is on, and
//! what the inbound rules for one program say. Nothing here changes a setting.

use std::path::Path;

use windows::core::Interface;
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, NetFwPolicy2, NET_FW_ACTION_ALLOW, NET_FW_PROFILE2_DOMAIN,
    NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC, NET_FW_PROFILE_TYPE2, NET_FW_RULE_DIR_IN,
};
use windows::Win32::System::Com::{CoCreateInstance, IDispatch, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Ole::IEnumVARIANT;
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_DISPATCH};

use super::com::ComScope;
use crate::{
    FirewallProfileStatus, FirewallRuleState, FirewallStatus, NetworkCategory, PlatformError,
};

/// `NET_FW_IP_PROTOCOL_TCP`.
const PROTOCOL_TCP: i32 = 6;
/// `NET_FW_IP_PROTOCOL_ANY`.
const PROTOCOL_ANY: i32 = 256;

fn com_err(what: &str) -> impl FnOnce(windows::core::Error) -> PlatformError + '_ {
    move |e| PlatformError::Unavailable {
        reason: format!("could not read the firewall ({what}): {e}"),
    }
}

/// The firewall's state for every profile, in the order Private, Public,
/// Domain, with the inbound TCP rules for `exe` on `port`. Read-only: only
/// getters are called.
pub fn read_firewall_status(exe: &Path, port: u16) -> Result<FirewallStatus, PlatformError> {
    let _com = ComScope::enter()?;
    // SAFETY: COM is initialised on this thread for the scope of `_com`, which
    // outlives every interface below.
    let policy: INetFwPolicy2 =
        unsafe { CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER) }
            .map_err(com_err("open the policy"))?;
    let rules = read_rules(&policy, exe)?;

    let profiles = [
        (NetworkCategory::Private, NET_FW_PROFILE2_PRIVATE),
        (NetworkCategory::Public, NET_FW_PROFILE2_PUBLIC),
        (NetworkCategory::Domain, NET_FW_PROFILE2_DOMAIN),
    ];
    let mut out = Vec::with_capacity(profiles.len());
    for (profile, kind) in profiles {
        out.push(FirewallProfileStatus {
            profile,
            firewall_on: profile_enabled(&policy, kind)?,
            rule: fold_rules(&rules, exe, port, kind.0),
        });
    }
    Ok(FirewallStatus { profiles: out })
}

fn profile_enabled(
    policy: &INetFwPolicy2,
    kind: NET_FW_PROFILE_TYPE2,
) -> Result<bool, PlatformError> {
    // SAFETY: a getter on a live interface; it takes the profile by value.
    let on = unsafe { policy.get_FirewallEnabled(kind) }.map_err(com_err("profile state"))?;
    Ok(on.as_bool())
}

/// The facts of every rule whose program is `exe`. Other programs' rules are
/// skipped after reading only their program name. A rule that cannot be read
/// is skipped.
fn read_rules(policy: &INetFwPolicy2, exe: &Path) -> Result<Vec<RuleFacts>, PlatformError> {
    // SAFETY: getters on live interfaces owned by this function.
    let enumerator: IEnumVARIANT = unsafe { policy.Rules().and_then(|r| r._NewEnum()) }
        .and_then(|unknown| unknown.cast())
        .map_err(com_err("list the rules"))?;

    let mut out = Vec::new();
    loop {
        let mut slot = [VARIANT::default()];
        let mut fetched = 0u32;
        // SAFETY: one zeroed VARIANT to fill and a valid out-pointer for the count.
        let hr = unsafe { enumerator.Next(&mut slot, &mut fetched) };
        if hr.is_err() {
            return Err(com_err("walk the rules")(hr.into()));
        }
        if fetched == 0 {
            break;
        }
        // SAFETY: the enumerator filled `slot[0]`; reading `vt` and, for
        // VT_DISPATCH, the `pdispVal` arm is valid. The clone takes its own
        // reference before `VariantClear` releases the VARIANT's.
        let dispatch: Option<IDispatch> = unsafe {
            let inner = &slot[0].Anonymous.Anonymous;
            let d = (inner.vt == VT_DISPATCH)
                .then(|| (*inner.Anonymous.pdispVal).clone())
                .flatten();
            let _ = VariantClear(&mut slot[0]);
            d
        };
        let Some(rule) = dispatch.and_then(|d| d.cast::<INetFwRule>().ok()) else {
            continue;
        };
        if let Some(facts) = rule_facts(&rule, exe) {
            out.push(facts);
        }
    }
    Ok(out)
}

/// The facts of `rule` when its program is `exe`, or `None` for another
/// program or a rule that cannot be read.
fn rule_facts(rule: &INetFwRule, exe: &Path) -> Option<RuleFacts> {
    // SAFETY: getters on a live interface; each returns an owned value.
    unsafe {
        let app = rule.ApplicationName().ok()?.to_string();
        if !same_program(Some(&app), exe) {
            return None;
        }
        Some(RuleFacts {
            enabled: rule.Enabled().ok()?.as_bool(),
            inbound: rule.Direction().ok()? == NET_FW_RULE_DIR_IN,
            app: Some(app),
            protocol: rule.Protocol().ok()?,
            local_ports: rule.LocalPorts().ok()?.to_string(),
            profiles: rule.Profiles().ok()?,
            allow: rule.Action().ok()? == NET_FW_ACTION_ALLOW,
        })
    }
}

fn same_program(app: Option<&str>, exe: &Path) -> bool {
    app.is_some_and(|a| a.eq_ignore_ascii_case(&exe.to_string_lossy()))
}

/// `"*"` (or empty: no port restriction) matches every port; otherwise the
/// port must appear as a plain number in the comma list. Ranges never match.
fn ports_match(local_ports: &str, port: u16) -> bool {
    let ports = local_ports.trim();
    ports.is_empty()
        || ports == "*"
        || ports
            .split(',')
            .any(|p| p.trim().parse::<u16>().is_ok_and(|p| p == port))
}

fn applies(rule: &RuleFacts, exe: &Path, port: u16, profile_bit: i32) -> bool {
    rule.enabled
        && rule.inbound
        && same_program(rule.app.as_deref(), exe)
        && matches!(rule.protocol, PROTOCOL_TCP | PROTOCOL_ANY)
        && ports_match(&rule.local_ports, port)
        && rule.profiles & profile_bit != 0
}

/// The facts about one firewall rule that decide whether it applies.
#[derive(Clone, Debug)]
pub(crate) struct RuleFacts {
    pub enabled: bool,
    pub inbound: bool,
    pub app: Option<String>,
    pub protocol: i32,
    pub local_ports: String,
    pub profiles: i32,
    pub allow: bool,
}

/// What the rules say for `exe` listening on TCP `port` within the profile
/// whose bit is `profile_bit`: any applying Block gives Blocked, otherwise any
/// Allow gives Allowed, otherwise NoRule.
pub(crate) fn fold_rules(
    rules: &[RuleFacts],
    exe: &Path,
    port: u16,
    profile_bit: i32,
) -> FirewallRuleState {
    let mut state = FirewallRuleState::NoRule;
    for rule in rules.iter().filter(|r| applies(r, exe, port, profile_bit)) {
        if !rule.allow {
            return FirewallRuleState::Blocked;
        }
        state = FirewallRuleState::Allowed;
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use FirewallRuleState::*;

    const EXE: &str = r"C:\Program Files\NetLantern\netlantern.exe";
    const PRIVATE: i32 = 2;
    const PUBLIC: i32 = 4;
    const DOMAIN: i32 = 1;
    const ALL: i32 = 0x7fff_ffff;

    fn rule(allow: bool) -> RuleFacts {
        RuleFacts {
            enabled: true,
            inbound: true,
            app: Some(EXE.to_string()),
            protocol: 6,
            local_ports: "*".to_string(),
            profiles: ALL,
            allow,
        }
    }

    fn fold(rules: &[RuleFacts], profile_bit: i32) -> FirewallRuleState {
        fold_rules(rules, Path::new(EXE), 47800, profile_bit)
    }

    #[test]
    fn block_wins_over_allow() {
        assert_eq!(fold(&[rule(true), rule(false)], PRIVATE), Blocked);
        assert_eq!(fold(&[rule(false), rule(true)], PRIVATE), Blocked);
        assert_eq!(fold(&[rule(true)], PRIVATE), Allowed);
        assert_eq!(fold(&[], PRIVATE), NoRule);
    }

    #[test]
    fn other_programs_ignored_case_insensitive_match() {
        let mut other = rule(false);
        other.app = Some(r"C:\Windows\System32\svchost.exe".to_string());
        let mut none = rule(false);
        none.app = None;
        assert_eq!(fold(&[other, none], PRIVATE), NoRule);

        let mut shouted = rule(true);
        shouted.app = Some(EXE.to_ascii_uppercase());
        assert_eq!(fold(&[shouted], PRIVATE), Allowed);
    }

    #[test]
    fn outbound_and_disabled_ignored() {
        let mut outbound = rule(false);
        outbound.inbound = false;
        let mut disabled = rule(false);
        disabled.enabled = false;
        assert_eq!(fold(&[outbound, disabled, rule(true)], PRIVATE), Allowed);
    }

    #[test]
    fn udp_only_rule_ignored() {
        let mut udp = rule(false);
        udp.protocol = 17;
        assert_eq!(fold(&[udp], PRIVATE), NoRule);

        let mut any = rule(true);
        any.protocol = 256;
        assert_eq!(fold(&[any], PRIVATE), Allowed);
    }

    #[test]
    fn local_ports_star_or_listed_port_only() {
        let with_ports = |ports: &str| {
            let mut r = rule(true);
            r.local_ports = ports.to_string();
            r
        };
        assert_eq!(fold(&[with_ports("*")], PRIVATE), Allowed);
        // An any-protocol rule has no port list at all: no restriction.
        assert_eq!(fold(&[with_ports("")], PRIVATE), Allowed);
        assert_eq!(fold(&[with_ports("80,47800,443")], PRIVATE), Allowed);
        assert_eq!(fold(&[with_ports("47800")], PRIVATE), Allowed);
        assert_eq!(fold(&[with_ports("80, 47800")], PRIVATE), Allowed);
        assert_eq!(fold(&[with_ports("80,443")], PRIVATE), NoRule);
        assert_eq!(fold(&[with_ports("478000")], PRIVATE), NoRule);
        // Ranges are not parsed.
        assert_eq!(fold(&[with_ports("47800-47809")], PRIVATE), NoRule);
    }

    #[test]
    fn profile_bit_filters() {
        let mut private_only = rule(false);
        private_only.profiles = PRIVATE;
        let mut public_and_domain = rule(true);
        public_and_domain.profiles = PUBLIC | DOMAIN;
        let rules = [private_only, public_and_domain];
        assert_eq!(fold(&rules, PRIVATE), Blocked);
        assert_eq!(fold(&rules, PUBLIC), Allowed);
        assert_eq!(fold(&rules, DOMAIN), Allowed);
    }
}
