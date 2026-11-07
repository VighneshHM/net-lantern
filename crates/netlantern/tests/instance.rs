#![cfg(windows)]

use std::time::Duration;

use netlantern::instance::{acquire, notify_open, serve_open_requests, Instance};
use tokio::io::AsyncWriteExt;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(2);
/// Long enough for the server to give up on a client that never writes.
const STALL_WAIT: Duration = Duration::from_secs(10);
/// How long the trigger channel must stay silent before a count is final.
const QUIET: Duration = Duration::from_millis(300);
const ERROR_PIPE_BUSY: i32 = 231;

/// A pipe name no other test (or running Net Lantern) uses.
fn unique_pipe(test: &str) -> String {
    format!(r"\\.\pipe\netlantern-test-{}-{}", std::process::id(), test)
}

/// Becomes the primary on `name` and serves it; every `open` request sends a
/// message on the returned channel.
fn serve(name: &str) -> mpsc::UnboundedReceiver<()> {
    let Instance::Primary(server) = acquire(name).unwrap() else {
        panic!("first must be primary")
    };
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(serve_open_requests(server, name.to_string(), move || {
        let _ = tx.send(());
    }));
    rx
}

/// Opens a raw client, retrying while every server instance is busy.
async fn connect(name: &str) -> NamedPipeClient {
    loop {
        match ClientOptions::new().open(name) {
            Ok(client) => return client,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
            Err(e) => panic!("cannot open pipe: {e}"),
        }
    }
}

#[tokio::test]
async fn first_acquire_is_primary_and_second_is_secondary() {
    let name = unique_pipe("acquire");
    let Instance::Primary(_server) = acquire(&name).unwrap() else {
        panic!("first must be primary")
    };
    assert!(matches!(acquire(&name).unwrap(), Instance::Secondary));
}

#[tokio::test]
async fn open_request_reaches_the_primary() {
    let name = unique_pipe("open");
    let Instance::Primary(server) = acquire(&name).unwrap() else {
        panic!("first must be primary")
    };
    let (tx, rx) = oneshot::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    tokio::spawn(serve_open_requests(server, name.clone(), move || {
        if let Some(tx) = tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }));

    timeout(WAIT, notify_open(&name)).await.unwrap().unwrap();
    timeout(WAIT, rx).await.unwrap().unwrap();
}

#[tokio::test]
async fn the_loop_keeps_serving_after_a_request() {
    let name = unique_pipe("twice");
    let mut opens = serve(&name);

    for _ in 0..2 {
        timeout(WAIT, notify_open(&name)).await.unwrap().unwrap();
        timeout(WAIT, opens.recv()).await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn a_line_that_is_not_open_does_not_trigger_it() {
    let name = unique_pipe("junk");
    let mut opens = serve(&name);

    // Wrong words, near misses, an empty line, junk lines short and
    // over-long (with no newline), and a client that connects and says
    // nothing before hanging up.
    let junk: [&[u8]; 7] = [
        b"hello\n",
        b"OPEN\n",
        b"open please\n",
        b"\n",
        &[b'x'; 200],
        &[b'x'; 4096],
        b"",
    ];
    for line in junk {
        let mut client = timeout(WAIT, connect(&name)).await.unwrap();
        client.write_all(line).await.unwrap();
        drop(client);
    }

    // Then one real request. Count every trigger until the channel has been
    // quiet for a while: exactly the real one must have fired `on_open`.
    timeout(WAIT, notify_open(&name)).await.unwrap().unwrap();
    timeout(WAIT, opens.recv()).await.unwrap().unwrap();
    let mut triggers = 1;
    while let Ok(Some(())) = timeout(QUIET, opens.recv()).await {
        triggers += 1;
    }
    assert_eq!(triggers, 1, "only the real `open` may trigger on_open");

    // And the loop is still alive.
    timeout(WAIT, notify_open(&name)).await.unwrap().unwrap();
    timeout(WAIT, opens.recv()).await.unwrap().unwrap();
}

#[tokio::test]
async fn a_client_that_never_writes_does_not_block_the_loop() {
    let name = unique_pipe("stall");
    let mut opens = serve(&name);

    // Holds its end open and stays silent.
    let _stalled = timeout(WAIT, connect(&name)).await.unwrap();

    timeout(WAIT, notify_open(&name)).await.unwrap().unwrap();
    timeout(STALL_WAIT, opens.recv()).await.unwrap().unwrap();
}

#[tokio::test]
async fn notify_open_without_a_primary_is_an_error() {
    let name = unique_pipe("nobody");
    assert!(timeout(WAIT, notify_open(&name)).await.unwrap().is_err());
}
