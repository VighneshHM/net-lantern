//! The plain-language copy for each finding.

use super::model::Finding;

/// Shown under a layer. `Healthy` is never a layer explanation, so it is empty.
pub fn explanation(f: Finding) -> &'static str {
    match f {
        Finding::EthernetOk => "Your laptop is connected by cable.",
        Finding::LinkOk => "Your laptop is connected and the signal is fine.",
        Finding::SignalHidden => {
            "Connected. Windows is hiding the signal details until location access is on."
        }
        Finding::LinkDetailsUnavailable => {
            "Connected. Windows didn't report signal details this time."
        }
        Finding::NotConnected => {
            "Your laptop isn't connected to Wi-Fi or a cable, so nothing else can be checked."
        }
        Finding::WeakSignal => "The signal is below \u{2212}70 dBm, which causes slow speeds and dropouts.",
        Finding::CrowdedChannel => {
            "Many nearby networks share your channel, so they take turns and slow each other down."
        }
        Finding::WeakAndCrowded => {
            "A weak signal and a busy channel together cause freezes and slow speeds."
        }
        Finding::RouterOk => "Your router answers quickly and reliably.",
        Finding::RouterIgnoresPing => {
            "Some routers ignore ping. This one answered at the network level, so the checks continue."
        }
        Finding::RouterUnreachable => {
            "The router didn't answer pings or an ARP request. It may be off or frozen, or the Wi-Fi link isn't really working."
        }
        Finding::RouterLossy => {
            "Lost messages between your laptop and router usually mean Wi-Fi interference or a weak signal."
        }
        Finding::RouterSlow => {
            "Replies took over 50 ms on average; on a home network they normally take a few milliseconds."
        }
        Finding::InternetOk => "Your router reaches the internet.",
        Finding::CaptivePortal => {
            "This network sends web traffic to a sign-in page first, so the internet is blocked until you sign in or accept its terms."
        }
        Finding::InternetDown => {
            "Your router works, but nothing on the internet answered: not 1.1.1.1, not 8.8.8.8, and not a web request."
        }
        Finding::InternetLossy => {
            "Your router answers reliably, but some messages beyond it go unanswered, which points to the modem or your provider."
        }
        Finding::InternetSlow => {
            "Replies from the internet took over 100 ms, which makes calls and games lag."
        }
        Finding::DnsOk => "Names like www.microsoft.com turn into addresses as they should.",
        Finding::NoDnsServer => {
            "Without a DNS server, names like www.microsoft.com can't be turned into addresses."
        }
        Finding::DnsServerProblem => {
            "Your network's DNS server didn't answer, but 1.1.1.1 did, so the internet works and only name lookups fail."
        }
        Finding::DnsUnreachable => {
            "Neither your network's DNS server nor 1.1.1.1 answered, although the internet answered. Something may be blocking DNS."
        }
        Finding::DnsPartial => "Your DNS server answered some lookups but not others.",
        Finding::WebsiteOk => "A secure website answered.",
        Finding::TlsIntercepted => {
            "A secure website's certificate wasn't trusted. A network filter, antivirus or hostile network may be reading encrypted traffic."
        }
        Finding::WebsiteUnreachable => "The lower layers work, but a secure website didn't load.",
        Finding::Healthy => "",
        Finding::Skipped => "Not checked because an earlier layer failed.",
    }
}

/// The verdict headline when this finding decides the result. `Skipped` is
/// never a verdict, so it is empty.
pub fn headline(f: Finding) -> &'static str {
    match f {
        Finding::EthernetOk => "Your cable connection is fine.",
        Finding::LinkOk => "Your Wi-Fi connection is fine.",
        Finding::SignalHidden => "Your Wi-Fi connection is up.",
        Finding::LinkDetailsUnavailable => "Your Wi-Fi connection is up.",
        Finding::NotConnected => "You're not connected to a network.",
        Finding::WeakSignal => "Your Wi-Fi signal is weak.",
        Finding::CrowdedChannel => "Your Wi-Fi channel is crowded.",
        Finding::WeakAndCrowded => "Your Wi-Fi signal is weak and the channel is crowded.",
        Finding::RouterOk => "Your router is fine.",
        Finding::RouterIgnoresPing => "Your router doesn't answer ping, but it is reachable.",
        Finding::RouterUnreachable => "Your laptop can't reach the router.",
        Finding::RouterLossy => "Some messages to your router are being lost.",
        Finding::RouterSlow => "Your router is slow to answer.",
        Finding::InternetOk => "Your internet connection is fine.",
        Finding::CaptivePortal => "Sign in to this network.",
        Finding::InternetDown => "Your internet connection is down.",
        Finding::InternetLossy => "Some messages to the internet are being lost.",
        Finding::InternetSlow => "Your internet connection is slow to respond.",
        Finding::DnsOk => "Name lookups are fine.",
        Finding::NoDnsServer => "Your laptop has no DNS server.",
        Finding::DnsServerProblem => "Your DNS server isn't working.",
        Finding::DnsUnreachable => "Name lookups are failing everywhere.",
        Finding::DnsPartial => "Some name lookups are failing.",
        Finding::WebsiteOk => "Websites load.",
        Finding::TlsIntercepted => "Something is intercepting secure connections.",
        Finding::WebsiteUnreachable => "Websites don't load.",
        Finding::Healthy => "Everything looks fine.",
        Finding::Skipped => "",
    }
}

/// What to do about it. `CrowdedChannel` has none here: the link judge takes
/// its fix from the congestion advice.
pub fn fix(f: Finding) -> Option<&'static str> {
    match f {
        Finding::SignalHidden => Some(
            "Turn on location access to see signal and channel details (see Wi-Fi quality below).",
        ),
        Finding::NotConnected => {
            Some("Connect to your Wi-Fi network, or check that Wi-Fi is turned on.")
        }
        Finding::WeakSignal => {
            Some("Move closer to the router, or remove obstacles between you and it.")
        }
        Finding::WeakAndCrowded => {
            Some("Move closer to the router, or join the 5 GHz network shown in the nearby list.")
        }
        Finding::RouterUnreachable => {
            Some("Restart the router (unplug it for 30 seconds), then reconnect.")
        }
        Finding::RouterLossy => Some("Move closer to the router, or restart it."),
        Finding::RouterSlow => Some(
            "Restart the router. If it stays slow, something on the network may be using all the bandwidth.",
        ),
        Finding::CaptivePortal => Some("Open any website in your browser to get the sign-in page."),
        Finding::InternetDown => Some(
            "Restart the router and modem. If that doesn't help, check your provider's status page or call them.",
        ),
        Finding::InternetLossy => {
            Some("Restart the modem. If it keeps happening, tell your provider.")
        }
        Finding::InternetSlow => Some(
            "Pause large downloads or streams on the network. If it stays slow, tell your provider.",
        ),
        Finding::NoDnsServer => Some(
            "Reconnect to the network. If it keeps happening, set DNS to 1.1.1.1 in your network settings.",
        ),
        Finding::DnsServerProblem => Some(
            "Restart the router. If that doesn't help, set DNS to 1.1.1.1 in your network or router settings.",
        ),
        Finding::DnsUnreachable => {
            Some("Restart the router. On a work or school network, ask its IT team.")
        }
        Finding::DnsPartial => {
            Some("Restart the router. If it keeps happening, set DNS to 1.1.1.1.")
        }
        Finding::TlsIntercepted => Some(
            "Don't enter passwords on this network. On a work network, ask IT whether they inspect traffic; otherwise check your antivirus' web protection.",
        ),
        Finding::WebsiteUnreachable => Some(
            "Turn off any VPN or proxy and try again. If it keeps happening, restart the router.",
        ),
        Finding::EthernetOk
        | Finding::LinkOk
        | Finding::LinkDetailsUnavailable
        | Finding::CrowdedChannel
        | Finding::RouterOk
        | Finding::RouterIgnoresPing
        | Finding::InternetOk
        | Finding::DnsOk
        | Finding::WebsiteOk
        | Finding::Healthy
        | Finding::Skipped => None,
    }
}
