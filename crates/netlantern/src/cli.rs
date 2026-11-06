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

    /// Do not open the dashboard in the browser on startup.
    #[arg(long)]
    pub no_browser: bool,

    /// Also accept this origin for cross-origin requests (the UI dev server).
    /// Repeatable.
    #[arg(long, value_name = "URL")]
    pub dev_origin: Vec<String>,
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
        assert!(c.dev_origin.is_empty());
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
    fn no_browser_flag() {
        assert!(Cli::parse_from(["netlantern", "--demo", "--no-browser"]).no_browser);
    }
}
