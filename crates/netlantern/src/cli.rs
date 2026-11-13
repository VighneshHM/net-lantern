//! Command-line arguments.

use std::path::PathBuf;

use clap::Parser;

/// Net Lantern: see what is on your Wi-Fi network.
#[derive(Debug, Parser)]
#[command(name = "netlantern", version)]
pub struct Cli {
    /// Run against a simulated network. `--demo` uses the built-in home
    /// network; `--demo=<file>` loads a scenario file instead.
    #[arg(long, num_args = 0..=1, require_equals = true, value_name = "FILE")]
    pub demo: Option<Option<PathBuf>>,

    /// Serve on exactly this port instead of the first free one in 47800-47809.
    /// Port 0 is refused: the URL and the Host check need the real port.
    #[arg(long, value_name = "PORT", value_parser = clap::value_parser!(u16).range(1..))]
    pub port: Option<u16>,

    /// Run the simulated network, and monitoring, this many times faster.
    #[arg(
        long,
        value_name = "N",
        requires = "demo",
        hide = true,
        value_parser = clap::value_parser!(u32).range(1..=600)
    )]
    pub time_scale: Option<u32>,

    /// Do not open the dashboard in the browser on startup.
    #[arg(long)]
    pub no_browser: bool,

    /// Show no desktop notifications (the tests and CI use this).
    #[arg(long, hide = true)]
    pub no_notify: bool,

    /// Also accept this origin for cross-origin requests (the UI dev server).
    /// Repeatable.
    #[arg(long, value_name = "URL")]
    pub dev_origin: Vec<String>,

    /// Keep Net Lantern's data in this folder instead of
    /// `%LOCALAPPDATA%\NetLantern`.
    #[arg(long, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    /// Run one quick scan, print the devices as a tab-separated table and
    /// exit. Starts no server and keeps nothing on disk.
    #[arg(long, conflicts_with_all = ["diagnose_once", "wifi_once"])]
    pub scan_once: bool,

    /// Run one diagnosis, print each step and the verdict, and exit. Starts
    /// no server and keeps nothing on disk.
    #[arg(long, conflicts_with_all = ["scan_once", "wifi_once"])]
    pub diagnose_once: bool,

    /// Print the current Wi-Fi connection and a scan of nearby networks, and
    /// exit. Starts no server and keeps nothing on disk.
    #[arg(long, conflicts_with_all = ["scan_once", "diagnose_once"])]
    pub wifi_once: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_flag_forms() {
        assert_eq!(Cli::parse_from(["netlantern", "--demo"]).demo, Some(None));
        assert_eq!(
            Cli::parse_from(["netlantern", "--demo=x.json"]).demo,
            Some(Some("x.json".into()))
        );
        let c = Cli::parse_from([
            "netlantern",
            "--port",
            "48000",
            "--dev-origin",
            "http://localhost:5173",
            "--dev-origin",
            "http://127.0.0.1:5173",
        ]);
        assert_eq!((c.port, c.dev_origin.len()), (Some(48000), 2));
    }

    #[test]
    fn everything_is_off_by_default() {
        let c = Cli::parse_from(["netlantern"]);
        assert_eq!(c.demo, None);
        assert_eq!(c.port, None);
        assert!(!c.no_browser);
        assert!(!c.no_notify);
        assert!(c.dev_origin.is_empty());
        assert_eq!(c.data_dir, None);
        assert!(!c.scan_once);
    }

    #[test]
    fn demo_needs_an_equals_sign_to_take_a_file() {
        // `--demo x.json` must not swallow the file: `x.json` is then an
        // unexpected positional argument, not the scenario.
        let err = Cli::try_parse_from(["netlantern", "--demo", "x.json"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn port_zero_is_rejected() {
        let err = Cli::try_parse_from(["netlantern", "--port", "0"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
        assert_eq!(
            Cli::parse_from(["netlantern", "--port", "1"]).port,
            Some(1),
            "the lowest valid port is accepted"
        );
    }

    #[test]
    fn time_scale_needs_demo_and_stays_in_range() {
        let err = Cli::try_parse_from(["netlantern", "--time-scale", "10"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
        for bad in ["0", "601"] {
            assert!(Cli::try_parse_from(["netlantern", "--demo", "--time-scale", bad]).is_err());
        }
        let c = Cli::parse_from(["netlantern", "--demo", "--time-scale", "600"]);
        assert_eq!(c.time_scale, Some(600));
    }

    #[test]
    fn no_browser_flag() {
        assert!(Cli::parse_from(["netlantern", "--demo", "--no-browser"]).no_browser);
    }

    #[test]
    fn no_notify_flag() {
        assert!(Cli::parse_from(["netlantern", "--demo", "--no-notify"]).no_notify);
    }

    #[test]
    fn scan_once_and_data_dir() {
        assert!(Cli::parse_from(["netlantern", "--scan-once"]).scan_once);
        assert_eq!(
            Cli::parse_from(["netlantern", "--data-dir", "D:\\x"]).data_dir,
            Some("D:\\x".into())
        );
        let c = Cli::parse_from(["netlantern", "--demo", "--scan-once"]);
        assert!(c.scan_once && c.demo.is_some());
    }

    #[test]
    fn once_flags_conflict() {
        for pair in [
            ["--diagnose-once", "--scan-once"],
            ["--diagnose-once", "--wifi-once"],
            ["--wifi-once", "--scan-once"],
        ] {
            let err = Cli::try_parse_from(["netlantern", pair[0], pair[1]]).unwrap_err();
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{pair:?}"
            );
        }
        let c = Cli::parse_from(["netlantern", "--demo", "--diagnose-once"]);
        assert!(c.diagnose_once && c.demo.is_some());
        assert!(Cli::parse_from(["netlantern", "--wifi-once"]).wifi_once);
    }
}
