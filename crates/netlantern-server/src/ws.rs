//! `GET /api/events`: the live event stream.
//!
//! The first message is `{"type":"hello","seq":N,"api_version":1}`, where `N`
//! is the seq of the last event published before the stream began. Every
//! later event follows as JSON text, in seq order, starting at `N + 1`. A
//! client that sees any other seq has missed events (a slow socket that fell
//! more than the bus capacity behind) and reloads its state over REST.
//!
//! The guard has already checked Host, Origin and the session before the
//! upgrade is accepted.

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use netlantern_core::{Event, EventBus, API_VERSION};
use serde::Serialize;
use tokio::sync::broadcast::{self, error::RecvError};

use crate::error::ApiError;
use crate::state::AppState;

pub(crate) async fn events(
    State(state): State<AppState>,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    match upgrade {
        // The callback runs once the connection is upgraded; only the feed
        // outlives it, so a dropped bus ends the stream.
        Ok(upgrade) => {
            upgrade.on_upgrade(move |socket| stream(socket, EventFeed::open(&state.bus)))
        }
        Err(_) => ApiError::bad_request("Expected a WebSocket upgrade request.").into_response(),
    }
}

#[derive(Serialize)]
struct Hello {
    #[serde(rename = "type")]
    kind: &'static str,
    seq: u64,
    api_version: u32,
}

/// Sends the hello, then forwards events until the client goes away, a send
/// fails, or the bus is dropped.
async fn stream(mut socket: WebSocket, mut feed: EventFeed) {
    let hello = Hello {
        kind: "hello",
        seq: feed.after,
        api_version: API_VERSION,
    };
    if !send_json(&mut socket, &hello).await {
        return;
    }
    loop {
        tokio::select! {
            event = feed.next() => match event {
                Some(event) => {
                    if !send_json(&mut socket, &event).await {
                        return;
                    }
                }
                None => return,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                // axum answers pings itself; anything else from the client is
                // ignored.
                Some(Ok(_)) => {}
            },
        }
    }
}

/// Whether `value` was sent as a JSON text message.
async fn send_json(socket: &mut WebSocket, value: &impl Serialize) -> bool {
    match serde_json::to_string(value) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => false,
    }
}

/// The events published after a known seq (`after`, the hello's seq).
struct EventFeed {
    rx: broadcast::Receiver<Event>,
    after: u64,
}

impl EventFeed {
    /// Subscribes first and reads the last seq second, so no event can fall
    /// between the two: anything published in the gap is both received and
    /// covered by `after`, and [`EventFeed::next`] drops it.
    fn open(bus: &EventBus) -> Self {
        let rx = bus.subscribe();
        let after = bus.last_seq();
        Self { rx, after }
    }

    /// The next event with a seq above `after`, or `None` once the bus is
    /// gone. A lagging receiver skips ahead; the client spots the gap.
    /// Cancel-safe: it only awaits `broadcast::Receiver::recv`.
    async fn next(&mut self) -> Option<Event> {
        loop {
            match self.rx.recv().await {
                Ok(event) if event.seq > self.after => return Some(event),
                Ok(_) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use netlantern_core::{EventKind, ScanId, ScanState};

    fn done(id: u64) -> EventKind {
        EventKind::ScanDone {
            scan_id: ScanId(id),
            state: ScanState::Done,
            error: None,
            isolated: false,
            devices_found: 0,
        }
    }

    /// The next event, if one is already waiting (never blocks).
    fn ready(feed: &mut EventFeed) -> Option<Event> {
        feed.next().now_or_never().expect("an event is ready")
    }

    #[test]
    fn a_feed_starts_after_the_last_published_seq() {
        let bus = EventBus::new(16);
        bus.publish(done(1));
        bus.publish(done(1));
        let mut feed = EventFeed::open(&bus);
        assert_eq!(feed.after, 2);

        bus.publish(done(2));
        assert_eq!(ready(&mut feed).unwrap().seq, 3);
    }

    #[test]
    fn events_already_covered_by_the_hello_are_dropped() {
        // The race `open` must survive: an event published between
        // `subscribe` and `last_seq` is both in the receiver and counted in
        // the hello's seq. Forwarding it would look like a repeat.
        let bus = EventBus::new(16);
        let rx = bus.subscribe();
        bus.publish(done(1));
        let mut feed = EventFeed {
            rx,
            after: bus.last_seq(),
        };

        bus.publish(done(2));
        let event = ready(&mut feed).unwrap();
        assert_eq!(event.seq, 2);
        assert_eq!(event.kind, done(2));
    }

    #[test]
    fn a_lagging_feed_skips_ahead_and_keeps_going() {
        let bus = EventBus::new(2);
        let mut feed = EventFeed::open(&bus);
        for _ in 0..5 {
            bus.publish(done(1));
        }
        // Seqs 1-3 were overwritten; the client sees the jump to 4 as a gap.
        assert_eq!(ready(&mut feed).unwrap().seq, 4);
        assert_eq!(ready(&mut feed).unwrap().seq, 5);
        bus.publish(done(1));
        assert_eq!(ready(&mut feed).unwrap().seq, 6);
    }

    #[test]
    fn a_feed_ends_when_the_bus_is_gone() {
        let bus = EventBus::new(16);
        let mut feed = EventFeed::open(&bus);
        drop(bus);
        assert!(ready(&mut feed).is_none());
    }
}
