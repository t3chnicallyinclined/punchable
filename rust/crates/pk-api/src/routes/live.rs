//! `GET /api/live` (`server.py` `live_socket`): the WebSocket the app keeps open to hear that something changed
//! (it refetches once per `change` event; `app/src/lib/api.ts`). The refusals in the Python's order: the 503 while
//! booting, `?backend=`, "this is a WebSocket endpoint" (400), the origin (403); then the upgrade. With auth on the
//! first message must be `{"type": "auth", "token": ...}` within 10 s (the token is never in the URL: URLs end up in
//! access logs), and a worker token cannot subscribe; a refusal is one `error` message, then close 4401. Then
//! `hello` (the rev served now, whether the subscription is up), the hub's events as they come, and a ping after 20 s
//! of quiet. What the client sends is ignored (its pings are answered by the socket layer). A client that stops
//! reading is dropped after 20 s of a send it does not take; one that fell behind the hub's backlog gets one
//! catch-up `change` (no `rev`) instead of the events it missed.

use std::time::Duration;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;

use crate::auth::Principal;
use crate::error::{ApiError, ApiResult};
use crate::query::Query;
use crate::state::AppState;

/// The first message (the token) must arrive within this.
const AUTH_WAIT: Duration = Duration::from_secs(10);
// The keep-alive ping (`PK_LIVE_PING_S`, 20 s) and the half-open limit (`PK_LIVE_IDLE_S`, 40 s) are settings
// (`ApiConfig::live_ping`, `live_idle`): a ping is sent every `live_ping` on its own deadline, never reset by an event
// (the app sends only its auth frame and pong replies, so only the pings make a healthy socket speak), and a client
// that has sent no frame for `live_idle` is half open and closed instead of living until TCP gives up.
/// The close code of a pod that is draining: "service restart", the client connects again.
const RESTARTING: u16 = 1012;
/// The largest client message read (`frame(limit=1 << 20)`).
const MAX_MESSAGE: usize = 1 << 20;
/// The largest first message (`frame(32768)` while the token is awaited): with auth on it is also the socket's frame
/// limit, so a stranger's 1 MiB first frame is refused before it is read whole (the client sends nothing else).
const MAX_AUTH_MESSAGE: usize = 32768;
/// A send the client does not take within this ends the session (a peer that stopped reading).
const SEND_WAIT: Duration = Duration::from_secs(20);
/// The close code of a refused token.
const UNAUTHORIZED: u16 = 4401;

/// The handler.
pub async fn live(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> ApiResult<Response> {
    let q = Query::parse(raw.as_deref());
    st.require_loaded()?;
    super::pick(&q)?;
    let is_upgrade = headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|u| u.eq_ignore_ascii_case("websocket"))
        && headers.contains_key("sec-websocket-key");
    if !is_upgrade {
        return Err(ApiError::bad_request("this is a WebSocket endpoint"));
    }
    // `origin and not self.origin_ok(origin)`: an origin that is not even text is not allowed either
    if let Some(o) = headers.get(header::ORIGIN)
        && o.to_str()
            .ok()
            .is_none_or(|o| !(o.is_empty() || crate::cors::origin_ok(o, &st.config().origins)))
    {
        return Err(ApiError::forbidden("origin not allowed"));
    }
    // the handshake headers are there but the connection cannot be upgraded (an old version, a test without a
    // socket): the same 400 as the Python's refusal of a request that is not a WebSocket one
    let ws = ws.map_err(|_| ApiError::bad_request("this is a WebSocket endpoint"))?;
    // with auth on the first frame is a token: nothing a client sends is larger than the auth message
    let limit = if st.verifier().is_some() {
        MAX_AUTH_MESSAGE
    } else {
        MAX_MESSAGE
    };
    Ok(ws
        .max_message_size(limit)
        .max_frame_size(limit)
        .on_upgrade(move |sock| session(st, sock)))
}

/// The open-sockets gauge, decremented however the session ends.
struct Client(prometheus::IntGauge);

impl Client {
    fn open(g: &prometheus::IntGauge) -> Self {
        g.inc();
        Self(g.clone())
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.0.dec();
    }
}

/// What the first message said (`msg.get("token") if ... type == "auth" else ""`), or why it could not be read.
enum First {
    Token(String),
    Failed(String),
    Gone,
}

async fn first_message(sock: &mut WebSocket) -> First {
    match tokio::time::timeout(AUTH_WAIT, sock.recv()).await {
        Err(_) => First::Failed("timed out".into()),
        // the client went away, or closed first
        Ok(None | Some(Ok(Message::Close(_)))) => First::Gone,
        Ok(Some(Err(e))) => First::Failed(e.to_string()),
        Ok(Some(Ok(Message::Text(t)))) if t.len() > MAX_AUTH_MESSAGE => First::Failed("frame too large".into()),
        Ok(Some(Ok(Message::Text(t)))) => match serde_json::from_str::<Value>(t.as_str()) {
            Ok(v) => First::Token(
                v.get("type")
                    .and_then(Value::as_str)
                    .filter(|k| *k == "auth")
                    .and_then(|_| v.get("token"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ),
            // `json.loads` on text that is not JSON: its own words differ; the detail is advisory
            Err(e) => First::Failed(e.to_string()),
        },
        // a binary, ping or pong frame: `msg = {}` and the empty token is refused
        Ok(Some(Ok(_))) => First::Token(String::new()),
    }
}

/// `str(e)[:80]`.
fn detail(e: &str) -> String {
    e.chars().take(80).collect()
}

/// One frame to the client, given up after [`SEND_WAIT`]: `false` ends the session.
async fn send(sock: &mut WebSocket, msg: Message) -> bool {
    tokio::time::timeout(SEND_WAIT, sock.send(msg))
        .await
        .is_ok_and(|r| r.is_ok())
}

async fn refuse(sock: &mut WebSocket, why: &str) {
    let msg = json!({"type": "error", "error": "unauthorized", "detail": detail(why)});
    if send(sock, Message::Text(msg.to_string().into())).await {
        send(
            sock,
            Message::Close(Some(CloseFrame {
                code: UNAUTHORIZED,
                reason: "".into(),
            })),
        )
        .await;
    }
}

async fn session(st: AppState, mut sock: WebSocket) {
    if let Some(v) = st.verifier() {
        let token = match first_message(&mut sock).await {
            First::Token(t) => t,
            First::Failed(why) => return refuse(&mut sock, &why).await,
            First::Gone => return,
        };
        match v.verify(&token).await {
            Ok(claims) => {
                if (Principal { claims }).is_worker() {
                    return refuse(&mut sock, "a worker token cannot subscribe").await;
                }
            }
            Err(e) => return refuse(&mut sock, &e.to_string()).await,
        }
    }
    let hub = st.live().clone();
    let mut rx = hub.subscribe();
    let mut drain = st.drain_watch();
    let (ping_every, idle_after) = (st.config().live_ping, st.config().live_idle);
    let mut last_rx = tokio::time::Instant::now();
    let mut next_ping = last_rx + ping_every;
    let _client = Client::open(&st.metrics().live_clients);
    let hello = json!({"type": "hello", "rev": st.data_now().map(|d| d.rev()), "live": hub.connected()});
    if !send(&mut sock, Message::Text(hello.to_string().into())).await {
        return;
    }
    loop {
        tokio::select! {
            ev = rx.recv() => {
                let text = match ev {
                    Ok(t) => t,
                    Err(RecvError::Lagged(n)) => {
                        // the backlog it fell behind on is dropped (a fresh receiver starts at now); one catch-up
                        // change makes it refetch once, which is all those events would have done
                        rx = rx.resubscribe();
                        Utf8Bytes::from(crate::live::catch_up(n, crate::worker::unix_now()))
                    }
                    Err(RecvError::Closed) => break,
                };
                if !send(&mut sock, Message::Text(text)).await {
                    break;
                }
            }
            msg = sock.recv() => match msg {
                // the client's frames: a close ends the session, anything else (its pings are answered by the
                // socket layer) is ignored, as the Python's reader ignores payloads
                None | Some(Err(_) | Ok(Message::Close(_))) => break,
                Some(Ok(_)) => last_rx = tokio::time::Instant::now(),
            },
            // no frame, not even a pong, for twice the ping interval: a half-open socket
            () = tokio::time::sleep_until(last_rx + idle_after) => break,
            // the pod is draining: tell the client to come back (1012), not a silent drop
            () = crate::state::drained(&mut drain) => {
                send(
                    &mut sock,
                    Message::Close(Some(CloseFrame { code: RESTARTING, reason: "".into() })),
                )
                .await;
                break;
            }
            // its own deadline: events do not move it, so a socket with events flowing still gets its pings and
            // answers with the pongs that keep `last_rx` fresh
            () = tokio::time::sleep_until(next_ping) => {
                next_ping = tokio::time::Instant::now() + ping_every;
                if !send(&mut sock, Message::Ping(Vec::new().into())).await {
                    break;
                }
            }
        }
    }
}
