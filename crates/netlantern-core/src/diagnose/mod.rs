//! The diagnosis model: wire types, probe types, a judge for each layer, the
//! verdict rule and the plain-language copy. Everything here is pure.

mod copy;
mod judge;
mod model;

pub use copy::{explanation, fix, headline};
pub use judge::{
    classify_portal, judge_dns, judge_internet, judge_link, judge_router, judge_website, skipped,
    verdict,
};
pub use model::{
    DiagnoseConfig, DiagnosisId, DiagnosisRun, DnsLookup, DnsProbe, Finding, InternetProbe, Layer,
    LayerResult, LayerStatus, LinkProbe, PingStats, PortalCheck, RouterProbe, RunState, TcpCheck,
    Verdict, WebsiteOutcome, WebsiteProbe, WifiReading,
};
