//! Spike: a Tauri window that hosts the Net Lantern server in-process and
//! points its webview at the server over loopback.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::thread::JoinHandle;

use netlantern_core::{Db, DiagnoseConfig, ScanConfig, Store};
use netlantern_platform::SimNetwork;
use netlantern_server::{bind, run, AccessPolicy, AppState, DEFAULT_PORTS};
use tauri::{RunEvent, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::oneshot;

const HOME_SCENARIO: &str = include_str!("../../../../scenarios/home.json");

/// The dashboard URL that logs the webview in with a one-time launch token.
fn login_url(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{port}/?t={token}")
}

/// A running in-process server: its login URL, a way to stop it, and its thread.
struct Hosted {
    url: String,
    shutdown: oneshot::Sender<()>,
    thread: JoinHandle<()>,
}

/// Binds a port, builds the simulated-network state and serves it on a
/// dedicated runtime thread. Returns once the server is listening.
fn start_server() -> Result<Hosted, String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<String, String>>();
    let (shutdown, shutdown_rx) = oneshot::channel::<()>();
    let thread = std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => return drop(ready_tx.send(Err(e.to_string()))),
        };
        rt.block_on(async move {
            let (listener, port) = match bind(DEFAULT_PORTS).await {
                Ok(v) => v,
                Err(e) => return drop(ready_tx.send(Err(e.to_string()))),
            };
            let sim = match SimNetwork::from_json(HOME_SCENARIO) {
                Ok(s) => Arc::new(s),
                Err(e) => return drop(ready_tx.send(Err(e.to_string()))),
            };
            let store = match Db::open_in_memory() {
                Ok(db) => Store::new(db),
                Err(e) => return drop(ready_tx.send(Err(e.to_string()))),
            };
            let state = AppState::new(
                sim.clone(),
                sim,
                store,
                ScanConfig::default(),
                DiagnoseConfig::default(),
                AccessPolicy::for_port(port, &[]),
            );
            let url = login_url(port, &state.auth.issue_launch_token());
            let _ = ready_tx.send(Ok(url));
            let _ = run(listener, state, async {
                let _ = shutdown_rx.await;
            })
            .await;
        });
    });
    let url = ready_rx.recv().map_err(|e| e.to_string())??;
    Ok(Hosted { url, shutdown, thread })
}

fn main() {
    let hosted = start_server().expect("server failed to start");
    let url: tauri::Url = hosted.url.parse().expect("valid url");
    let Hosted { shutdown, thread, .. } = hosted;
    let mut stop = Some((shutdown, thread));

    let app = tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("Net Lantern")
                .inner_size(1200.0, 800.0)
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error building the app");

    app.run(move |_, event| {
        if let RunEvent::Exit = event {
            if let Some((shutdown, thread)) = stop.take() {
                let _ = shutdown.send(());
                let _ = thread.join();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_carries_port_and_token() {
        assert_eq!(login_url(47800, "abc"), "http://127.0.0.1:47800/?t=abc");
    }

    #[test]
    fn server_starts_and_stops() {
        let hosted = start_server().unwrap();
        assert!(hosted.url.starts_with("http://127.0.0.1:478"));
        assert!(hosted.url.contains("/?t="));
        let _ = hosted.shutdown.send(());
        hosted.thread.join().unwrap();
    }
}
