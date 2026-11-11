//! The diagnosis: wire types, probe types, a judge for each layer, the verdict
//! rule and the plain-language copy (all pure), plus the probes that gather
//! what the judges need and the runner that checks the layers in turn.

mod copy;
mod judge;
mod manager;
mod model;
mod probe;

pub use copy::{explanation, fix, headline};
pub use judge::{
    classify_portal, judge_dns, judge_internet, judge_link, judge_router, judge_website, skipped,
    verdict,
};
pub use manager::{DiagnoseManager, DiagnoseStart};
pub use model::{
    DiagnoseConfig, DiagnosisId, DiagnosisRun, DnsLookup, DnsProbe, Finding, InternetProbe, Layer,
    LayerResult, LayerStatus, LinkProbe, PingStats, PortalCheck, RouterProbe, RunState, TcpCheck,
    Verdict, WebsiteOutcome, WebsiteProbe, WifiReading,
};
pub use probe::{gather_dns, gather_internet, gather_link, gather_router, gather_website};
