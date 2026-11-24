use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::{Duration, Instant};

use envoy_proxy_dynamic_modules_rust_sdk::{EnvoyHttpFilter, EnvoyHttpFilterScheduler as _, abi};
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header, uri};
use tungstenite::extensions::{Extensions, ExtensionsConfig, compression::deflate::DeflateConfig};
use tungstenite::handshake::derive_accept_key;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tungstenite::{Message, Utf8Bytes, WebSocket};

use crate::eventbridge::EventBridge;
use crate::types::Scope;

use executor::{
    Body, EVENT_ID_REQUEST, RecvFuture, ResponseBodyEvent, ResponseStartEvent, SendEvent,
};
pub(crate) use executor::{
    EVENT_ID_CLOSE_TIMEOUT, EVENT_ID_PING, EVENT_ID_PING_TIMEOUT, WebSocketExecutor,
};
use keepalive::{PingTracker, TimeoutAction};
use stream::EnvoyStream;
use timer::Timer;

mod executor;
mod keepalive;
mod stream;
mod timer;

/// How long to wait for an HTTP/2 or HTTP/3 client to end its stream after
/// the close handshake before ending ours anyways.
const HALF_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// The WebSocket settings shared by all streams of a filter.
pub(crate) struct Config {
    executor: WebSocketExecutor,
    max_message_size: usize,
    deflate: Option<DeflateConfig>,
    /// How often to ping clients, or never if unset.
    ping_interval: Option<Duration>,
    /// How long a client may send nothing after a ping before it is closed, or
    /// forever if unset.
    ping_timeout: Option<Duration>,
    timer: Timer,
}

impl Config {
    pub(crate) fn new(
        executor: WebSocketExecutor,
        max_message_size: usize,
        deflate: Option<DeflateConfig>,
        ping_interval: Option<Duration>,
        ping_timeout: Option<Duration>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            executor,
            max_message_size,
            deflate,
            ping_interval,
            ping_timeout,
            timer: Timer::new()?,
        })
    }
}

/// Whether the request is a WebSocket opening handshake. An HTTP/1 handshake
/// must have a key while HTTP/2 and HTTP/3 ones never do. Other requests,
/// including invalid handshakes, are served as HTTP.
pub(crate) fn is_handshake(scope: &Scope) -> bool {
    let header = |name: &HeaderName| {
        scope
            .headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    };
    header(&header::UPGRADE).is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"))
        && header(&header::SEC_WEBSOCKET_VERSION).is_some_and(|v| v.as_bytes() == b"13")
        && (scope.http_version >= http::Version::HTTP_2
            || header(&header::SEC_WEBSOCKET_KEY).is_some())
}

enum State {
    /// Waiting for the application to accept or deny the connection.
    Handshake {
        /// The `Sec-WebSocket-Accept` value for an HTTP/1 handshake.
        accept: Option<HeaderValue>,
        /// The client's `Sec-WebSocket-Extensions` offers.
        extension_offers: HeaderMap,
    },
    Accepted(Box<WebSocket<EnvoyStream>>),
    /// Streaming the body of an HTTP response denying the connection.
    Responding,
    Done,
}

/// A WebSocket connection on one HTTP stream.
///
/// Calls into Envoy can re-enter the filter, so no `RefCell` borrow is held
/// across them.
pub(crate) struct WebSocketStream {
    config: Arc<Config>,
    state: RefCell<State>,
    /// Our handle on the bytes the WebSocket reads and writes.
    io: EnvoyStream,
    close_frame: RefCell<Option<CloseFrame>>,
    connect_sent: Cell<bool>,

    recv_bridge: EventBridge<RecvFuture>,
    send_bridge: EventBridge<SendEvent>,

    /// Whether the client ended its request stream.
    request_ended: Cell<bool>,
    /// Whether to wait for the client to end its stream after the close
    /// handshake before ending ours, true for HTTP/2 and HTTP/3. Envoy resets a
    /// stream whose response ends before its request, discarding response data
    /// still waiting on flow control such as our Close frame. HTTP/1 clients
    /// instead wait for the server to close the connection first (RFC 6455
    /// 7.1.1), which Envoy does when the response ends.
    wait_for_half_close: bool,
    /// Whether the close handshake is done and the response is waiting to end.
    closing: Cell<bool>,
    response_ended: Cell<bool>,
    ping: RefCell<PingTracker>,
}

impl WebSocketStream {
    /// Starts the application for a WebSocket handshake request.
    pub(crate) fn start(
        config: Arc<Config>,
        envoy_filter: &mut impl EnvoyHttpFilter,
        mut scope: Scope,
        end_of_stream: bool,
    ) -> Self {
        let mut accept = None;
        let mut extension_offers = HeaderMap::new();
        for (name, value) in &scope.headers {
            if name == header::SEC_WEBSOCKET_KEY {
                accept = HeaderValue::from_str(&derive_accept_key(value.as_bytes())).ok();
            } else if name == header::SEC_WEBSOCKET_EXTENSIONS {
                extension_offers.append(header::SEC_WEBSOCKET_EXTENSIONS, value.clone());
            }
        }
        let subprotocols = parse_subprotocols(&scope.headers);
        let wait_for_half_close = scope.http_version >= http::Version::HTTP_2;
        scope.scheme = if scope.scheme == uri::Scheme::HTTPS {
            uri::Scheme::try_from("wss").unwrap()
        } else {
            uri::Scheme::try_from("ws").unwrap()
        };

        let stream = Self {
            config,
            state: RefCell::new(State::Handshake {
                accept,
                extension_offers,
            }),
            io: EnvoyStream::default(),
            close_frame: RefCell::new(None),
            connect_sent: Cell::new(false),
            recv_bridge: EventBridge::new(),
            send_bridge: EventBridge::new(),
            request_ended: Cell::new(end_of_stream),
            wait_for_half_close,
            closing: Cell::new(false),
            response_ended: Cell::new(false),
            ping: RefCell::new(PingTracker::default()),
        };
        stream.config.executor.execute_app(
            scope,
            subprotocols,
            stream.recv_bridge.clone(),
            stream.send_bridge.clone(),
            Box::new(envoy_filter.new_scheduler()),
        );
        stream
    }

    pub(crate) fn on_request_body(
        &self,
        envoy_filter: &mut impl EnvoyHttpFilter,
        end_of_stream: bool,
    ) {
        self.ping.borrow_mut().on_client_data();
        if end_of_stream {
            self.request_ended.set(true);
        }
        self.handle_read(envoy_filter);
    }

    pub(crate) fn on_request_trailers(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        self.request_ended.set(true);
        self.handle_read(envoy_filter);
    }

    pub(crate) fn on_request_event(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        if matches!(*self.state.borrow(), State::Handshake { .. }) {
            if !self.connect_sent.get()
                && let Some(future) = self.recv_bridge.get()
            {
                self.connect_sent.set(true);
                self.config.executor.handle_recv_future_connect(future);
            }
            return;
        }
        self.handle_read(envoy_filter);
    }

    pub(crate) fn on_close_timeout(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        if self.closing.get() {
            self.end_response(envoy_filter);
        }
    }

    pub(crate) fn on_ping(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        if self.close_frame.borrow().is_some() || self.response_ended.get() {
            return;
        }
        // Fails once the WebSocket has read the client's Close, even if the
        // application hasn't received it yet.
        let sent = self
            .with_websocket(|websocket| websocket.send(Message::Ping(Default::default())).is_ok());
        if sent != Some(true) {
            return;
        }
        self.flush(envoy_filter);
        if let Some(timeout) = self.config.ping_timeout {
            let schedule_timeout = self.ping.borrow_mut().on_ping(Instant::now());
            if schedule_timeout {
                self.schedule_ping_timeout(envoy_filter, timeout);
            }
        }
        self.schedule_ping(envoy_filter);
    }

    pub(crate) fn on_ping_timeout(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        // Only scheduled with a timeout.
        let timeout = self.config.ping_timeout.unwrap();
        let action = self.ping.borrow_mut().on_timeout(Instant::now(), timeout);
        match action {
            TimeoutAction::None => return,
            TimeoutAction::Reschedule(delay) => {
                self.schedule_ping_timeout(envoy_filter, delay);
                return;
            }
            TimeoutAction::Close => {}
        }
        if self.close_frame.borrow().is_some() {
            // Already closing, so there is nothing more to tell the client.
            self.end_response(envoy_filter);
            return;
        }
        self.with_websocket(|websocket| {
            let _ = websocket.close(Some(CloseFrame {
                code: CloseCode::Error,
                reason: Utf8Bytes::from_static("keepalive ping timeout"),
            }));
        });
        self.flush(envoy_filter);
        // The client is likely gone, so we don't wait for its Close frame.
        self.abnormal_close(envoy_filter);
    }

    fn schedule_ping(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        if let Some(interval) = self.config.ping_interval {
            self.config.timer.schedule(
                interval,
                Box::new(envoy_filter.new_scheduler()),
                EVENT_ID_PING,
            );
        }
    }

    fn schedule_ping_timeout(&self, envoy_filter: &mut impl EnvoyHttpFilter, delay: Duration) {
        self.config.timer.schedule(
            delay,
            Box::new(envoy_filter.new_scheduler()),
            EVENT_ID_PING_TIMEOUT,
        );
    }

    pub(crate) fn on_stream_complete(&self) {
        // Dropping any pending receives resolves them to a disconnect.
        self.cleanup();
        self.response_ended.set(true);
        *self.state.borrow_mut() = State::Done;
    }

    fn handle_read(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        if self.closing.get() {
            // The close handshake is done, so anything else from the client is
            // discarded while we wait for it to end its stream.
            drain_request_body(envoy_filter);
            if self.request_ended.get() {
                self.end_response(envoy_filter);
            }
            return;
        }
        if matches!(*self.state.borrow(), State::Accepted(_)) {
            self.read_web_socket(envoy_filter);
        }
    }

    pub(crate) fn process_send_events(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        if matches!(*self.state.borrow(), State::Handshake { .. }) {
            let Some(event) = self.send_bridge.get() else {
                return;
            };
            self.process_handshake_event(envoy_filter, event);
            return;
        }

        self.send_bridge.process(|event| {
            if self.response_ended.get() {
                return;
            }
            let responding = matches!(*self.state.borrow(), State::Responding);
            if responding {
                self.process_denial_event(envoy_filter, event);
                return;
            }
            match event {
                SendEvent::SendMessage(event) => {
                    if self.close_frame.borrow().is_some() {
                        // Dropping the future fails the send as disconnected.
                        return;
                    }
                    let message = match event.body {
                        Body::Text(text) => Message::Text(text),
                        Body::Bytes(bytes) => Message::Binary(bytes),
                    };
                    self.with_websocket(|websocket| {
                        let _ = websocket.send(message);
                    });
                    self.flush(envoy_filter);
                    self.config.executor.handle_send_future(event.future);
                }
                SendEvent::Close { code, reason } => {
                    self.start_close(
                        envoy_filter,
                        CloseFrame {
                            code: CloseCode::from(code),
                            reason,
                        },
                    );
                }
                SendEvent::Exception => {
                    self.start_close(
                        envoy_filter,
                        CloseFrame {
                            code: CloseCode::Error,
                            reason: Utf8Bytes::default(),
                        },
                    );
                }
                SendEvent::Abort => {
                    // Don't abort if we're already closing gracefully.
                    if self.close_frame.borrow().is_none() {
                        self.reset(
                            envoy_filter,
                            "WebSocket application returned without closing",
                        );
                    }
                }
                // SendCallable only sends denial events before accepting.
                SendEvent::Accept(_)
                | SendEvent::ResponseStart(..)
                | SendEvent::ResponseBody(_) => {}
            }
        });
    }

    fn process_handshake_event(&self, envoy_filter: &mut impl EnvoyHttpFilter, event: SendEvent) {
        match event {
            SendEvent::Accept(event) => {
                let State::Handshake {
                    accept,
                    extension_offers,
                } = std::mem::replace(&mut *self.state.borrow_mut(), State::Done)
                else {
                    return;
                };
                let mut headers = HeaderMap::new();
                headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
                headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
                if let Some(accept) = accept {
                    headers.insert(header::SEC_WEBSOCKET_ACCEPT, accept);
                }
                // Negotiate permessage-deflate against the client's offers,
                // writing the agreed extension header into the response. A
                // malformed extensions header falls back to no compression.
                let mut extensions_config = ExtensionsConfig::default();
                extensions_config.permessage_deflate = self.config.deflate;
                let extensions = extensions_config
                    .negotiate_response(&extension_offers, &mut headers)
                    .unwrap_or_else(|_| Extensions::default());
                if let Some(subprotocol) = &event.subprotocol
                    && let Ok(value) = HeaderValue::from_str(subprotocol)
                {
                    headers.append(header::SEC_WEBSOCKET_PROTOCOL, value);
                }
                for (name, value) in event.headers {
                    headers.append(name, value);
                }
                let ws_config =
                    WebSocketConfig::default().max_message_size(Some(self.config.max_message_size));
                *self.state.borrow_mut() =
                    State::Accepted(Box::new(WebSocket::from_raw_socket_with_extensions(
                        self.io.clone(),
                        Role::Server,
                        Some(ws_config),
                        extensions,
                    )));
                // Envoy converts the 101 to a 200 for HTTP/2 and HTTP/3.
                send_headers(
                    envoy_filter,
                    StatusCode::SWITCHING_PROTOCOLS,
                    &headers,
                    false,
                );
                // In case there were any recv tasks started before accepting.
                envoy_filter.new_scheduler().commit(EVENT_ID_REQUEST);
                self.schedule_ping(envoy_filter);
                self.config.executor.handle_send_future(event.future);
            }
            SendEvent::Close { code, reason } => {
                *self.close_frame.borrow_mut() = Some(CloseFrame {
                    code: CloseCode::from(code),
                    reason,
                });
                *self.state.borrow_mut() = State::Done;
                self.response_ended.set(true);
                let mut headers = HeaderMap::new();
                headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
                send_headers(envoy_filter, StatusCode::FORBIDDEN, &headers, true);
            }
            SendEvent::ResponseStart(start, body) => {
                *self.state.borrow_mut() = State::Responding;
                self.send_denial_start(envoy_filter, start, body);
            }
            SendEvent::Abort => {
                *self.state.borrow_mut() = State::Done;
                self.reset(
                    envoy_filter,
                    "WebSocket application returned before accepting",
                );
            }
            SendEvent::Exception => {
                // Mainly to handle if the application created a task that calls recv, after the app
                // finished with exception. It's not a meaningful app in the real-world.
                *self.close_frame.borrow_mut() = Some(CloseFrame {
                    code: CloseCode::Error,
                    reason: Utf8Bytes::default(),
                });
                *self.state.borrow_mut() = State::Done;
                self.response_ended.set(true);
                // Follow uvicorn's behavior which returns 500 for exceptions before handshake.
                envoy_filter.send_response(
                    500,
                    &[("content-type", b"text/plain; charset=utf-8")],
                    Some(b"Internal Server Error"),
                    None,
                );
            }
            // SendCallable only allows these after accepting or denying.
            SendEvent::SendMessage(_) | SendEvent::ResponseBody(_) => {}
        }
    }

    fn send_denial_start(
        &self,
        envoy_filter: &mut impl EnvoyHttpFilter,
        start: ResponseStartEvent,
        body: ResponseBodyEvent,
    ) {
        let end_stream = body.future.is_none();
        let mut headers = HeaderMap::new();
        for (name, value) in start.headers {
            headers.append(name, value);
        }
        if end_stream
            && !headers.contains_key(header::CONTENT_LENGTH)
            && start.status != StatusCode::NO_CONTENT
            && start.status != StatusCode::NOT_MODIFIED
        {
            headers.insert(header::CONTENT_LENGTH, HeaderValue::from(body.body.len()));
        }
        match body.future {
            Some(future) => self.config.executor.handle_send_future(future),
            None => {
                *self.state.borrow_mut() = State::Done;
                self.response_ended.set(true);
            }
        }
        if end_stream && body.body.is_empty() {
            send_headers(envoy_filter, start.status, &headers, true);
        } else {
            send_headers(envoy_filter, start.status, &headers, false);
            envoy_filter.send_response_data(&body.body, end_stream);
        }
    }

    fn process_denial_event(&self, envoy_filter: &mut impl EnvoyHttpFilter, event: SendEvent) {
        match event {
            SendEvent::ResponseBody(body) => {
                let end_stream = body.future.is_none();
                match body.future {
                    Some(future) => self.config.executor.handle_send_future(future),
                    None => {
                        *self.state.borrow_mut() = State::Done;
                        self.response_ended.set(true);
                    }
                }
                envoy_filter.send_response_data(&body.body, end_stream);
            }
            SendEvent::Exception | SendEvent::Abort => {
                // The response head is already sent, so reset to signal the
                // body is incomplete.
                *self.state.borrow_mut() = State::Done;
                self.reset(envoy_filter, "Incomplete WebSocket denial response");
            }
            // SendCallable only allows response body events while responding.
            _ => {}
        }
    }

    fn read_web_socket(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        let close_frame = self.close_frame.borrow().clone();
        if let Some(close_frame) = close_frame {
            self.disconnect_receivers(&close_frame);
            self.finish_close(envoy_filter);
            return;
        }

        loop {
            if self.recv_bridge.is_empty() {
                // Without a pending receive, we only read pings and pongs so
                // pings are answered and pongs don't pile up in Envoy. Anything
                // else stays in Envoy for backpressure, including a Close, which
                // would fail the application's sends before it has seen it.
                match self.io.next_frame_is_ping_or_pong() {
                    Some(true) => {}
                    Some(false) => return,
                    None => {
                        if self.io.read_block(envoy_filter) == 0 {
                            return;
                        }
                        continue;
                    }
                }
            }
            let Some(message) = self.with_websocket(|websocket| websocket.read()) else {
                return;
            };
            self.flush(envoy_filter);
            match message {
                Ok(Message::Binary(bytes)) => self.deliver(Body::Bytes(bytes)),
                Ok(Message::Text(text)) => self.deliver(Body::Text(text)),
                Ok(Message::Close(frame)) => {
                    let close_frame = frame.unwrap_or(CloseFrame {
                        code: CloseCode::Status,
                        reason: Utf8Bytes::default(),
                    });
                    *self.close_frame.borrow_mut() = Some(close_frame.clone());
                    self.disconnect_receivers(&close_frame);
                    self.cleanup();
                    self.finish_close(envoy_filter);
                    return;
                }
                Ok(Message::Ping(_)) => {
                    // Reading only queues the pong, so send it in case we stop
                    // reading.
                    self.with_websocket(|websocket| {
                        let _ = websocket.flush();
                    });
                    self.flush(envoy_filter);
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(ref e))
                    if e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    // Need more bytes to parse a frame. Pull the next block from
                    // Envoy if available, or wait otherwise.
                    if self.io.read_block(envoy_filter) == 0 {
                        // Without a pending receive, the disconnect waits for the
                        // application to receive.
                        if self.request_ended.get() && !self.recv_bridge.is_empty() {
                            self.abnormal_close(envoy_filter);
                        }
                        return;
                    }
                }
                Err(e) => {
                    let code = match e {
                        tungstenite::Error::Protocol(_) => CloseCode::Protocol,
                        tungstenite::Error::Utf8(_) => CloseCode::Invalid,
                        tungstenite::Error::Capacity(_) => CloseCode::Size,
                        _ => CloseCode::Error,
                    };
                    let close_frame = CloseFrame {
                        code,
                        reason: Utf8Bytes::default(),
                    };
                    self.disconnect_receivers(&close_frame);
                    self.cleanup();
                    self.start_close(envoy_filter, close_frame);
                    return;
                }
            }
        }
    }

    fn deliver(&self, body: Body) {
        if let Some(future) = self.recv_bridge.get() {
            self.config
                .executor
                .handle_recv_future_message(body, future);
        }
    }

    /// Sends our Close frame and finishes the close handshake as far as the
    /// client's data allows.
    fn start_close(&self, envoy_filter: &mut impl EnvoyHttpFilter, close_frame: CloseFrame) {
        self.with_websocket(|websocket| {
            let _ = websocket.close(Some(close_frame.clone()));
        });
        *self.close_frame.borrow_mut() = Some(close_frame);
        self.flush(envoy_filter);
        self.finish_close(envoy_filter);
    }

    fn finish_close(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        // Closing a WebSocket exchanges Close frames, which may cross request
        // body events, so we read and write as much as we can each time.
        loop {
            let Some(message) = self.with_websocket(|websocket| websocket.read()) else {
                return;
            };
            self.flush(envoy_filter);
            // We shouldn't see any more real messages but it's fine to ignore them.
            match message {
                Ok(_) => {}
                Err(tungstenite::Error::Io(ref e))
                    if e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    if self.io.read_block(envoy_filter) == 0 {
                        if self.request_ended.get() {
                            // The client ended without completing the close
                            // handshake, so there is nothing to wait for.
                            self.end_response(envoy_filter);
                        }
                        return;
                    }
                }
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    self.close_complete(envoy_filter);
                    return;
                }
                Err(_) => {
                    self.reset(envoy_filter, "WebSocket protocol error while closing");
                    return;
                }
            }
        }
    }

    fn close_complete(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        self.closing.set(true);
        self.cleanup();
        drain_request_body(envoy_filter);
        if !self.wait_for_half_close || self.request_ended.get() {
            self.end_response(envoy_filter);
        } else {
            self.config.timer.schedule(
                HALF_CLOSE_TIMEOUT,
                Box::new(envoy_filter.new_scheduler()),
                EVENT_ID_CLOSE_TIMEOUT,
            );
        }
    }

    /// Handles the client ending its stream without a close handshake.
    fn abnormal_close(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        let close_frame = CloseFrame {
            code: CloseCode::Abnormal,
            reason: Utf8Bytes::default(),
        };
        self.disconnect_receivers(&close_frame);
        *self.close_frame.borrow_mut() = Some(close_frame);
        self.cleanup();
        self.end_response(envoy_filter);
    }

    fn end_response(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        if self.response_ended.replace(true) {
            return;
        }
        self.closing.set(false);
        *self.state.borrow_mut() = State::Done;
        envoy_filter.send_response_data(&[], true);
    }

    fn reset(&self, envoy_filter: &mut impl EnvoyHttpFilter, details: &str) {
        if self.response_ended.replace(true) {
            return;
        }
        self.cleanup();
        *self.state.borrow_mut() = State::Done;
        envoy_filter.reset_stream(
            abi::envoy_dynamic_module_type_http_filter_stream_reset_reason::LocalReset,
            details,
        );
    }

    fn disconnect_receivers(&self, close_frame: &CloseFrame) {
        self.recv_bridge.process(|future| {
            self.config.executor.handle_recv_future_disconnect(
                close_frame.code.into(),
                close_frame.reason.clone(),
                future,
            );
        });
    }

    /// Runs `f` with the accepted WebSocket, returning `None` if there is none.
    fn with_websocket<R>(&self, f: impl FnOnce(&mut WebSocket<EnvoyStream>) -> R) -> Option<R> {
        match &mut *self.state.borrow_mut() {
            State::Accepted(websocket) => Some(f(websocket)),
            _ => None,
        }
    }

    /// Sends what the WebSocket has written to the client.
    fn flush(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        if self.response_ended.get() {
            return;
        }
        if let Some(bytes) = self.io.take_written() {
            envoy_filter.send_response_data(&bytes, false);
        }
    }

    fn cleanup(&self) {
        self.recv_bridge.close();
        self.send_bridge.close();
    }
}

/// Discards the readable request body.
fn drain_request_body(envoy_filter: &mut impl EnvoyHttpFilter) {
    let buffered = envoy_filter.get_buffered_request_body_size();
    if buffered > 0 {
        envoy_filter.drain_buffered_request_body(buffered);
    }
    let received = envoy_filter.get_received_request_body_size();
    if received > 0 {
        envoy_filter.drain_received_request_body(received);
    }
}

fn send_headers(
    envoy_filter: &mut impl EnvoyHttpFilter,
    status: StatusCode,
    headers: &HeaderMap,
    end_stream: bool,
) {
    let mut envoy_headers: Vec<(&str, &[u8])> = Vec::with_capacity(headers.len() + 1);
    envoy_headers.push((":status", status.as_str().as_bytes()));
    for (name, value) in headers {
        envoy_headers.push((name.as_str(), value.as_bytes()));
    }
    envoy_filter.send_response_headers(&envoy_headers, end_stream);
}

/// Parses the comma-separated subprotocols offered by the client across any
/// number of `Sec-WebSocket-Protocol` headers, in offer order.
fn parse_subprotocols(headers: &[(HeaderName, HeaderValue)]) -> Vec<String> {
    headers
        .iter()
        .filter(|(name, _)| name == header::SEC_WEBSOCKET_PROTOCOL)
        .filter_map(|(_, value)| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|protocol| !protocol.is_empty())
        .map(str::to_owned)
        .collect()
}
