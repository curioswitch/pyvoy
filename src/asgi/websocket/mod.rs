use envoy_proxy_dynamic_modules_rust_sdk::{
    CatchUnwind, EnvoyNetworkFilter, EnvoyNetworkFilterScheduler as _, NetworkFilter,
    NetworkFilterConfig, abi, envoy_log_error,
};
use http::{HeaderName, HeaderValue, header, uri};
use pyo3::Python;
use pyo3::types::PyTracebackMethods as _;
use std::sync::Arc;
use tungstenite::extensions::{Extensions, ExtensionsConfig, compression::deflate::DeflateConfig};
use tungstenite::handshake::machine::{HandshakeMachine, RoundResult, StageResult};
use tungstenite::handshake::server::{create_response, write_response};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tungstenite::{Message, Utf8Bytes, WebSocket};

use crate::asgi::python::EVENT_ID_REQUEST;
use crate::asgi::shared::ExecutorHandles;
use crate::asgi::websocket::executor::{
    Body, EVENT_ID_RESPONSE, RecvFuture, SendEvent, WebSocketExecutor,
};
use crate::asgi::websocket::stream::EnvoyStream;
use crate::eventbridge::EventBridge;
use crate::types::*;

mod executor;
mod stream;

pub struct Config {
    executor: WebSocketExecutor,
    handles: Option<ExecutorHandles>,
    max_message_size: usize,
    compression: bool,
}

impl Config {
    pub fn new(
        app: &str,
        constants: Arc<Constants>,
        worker_threads: usize,
        enable_lifespan: Option<bool>,
        max_message_size: usize,
        compression: bool,
    ) -> Option<Self> {
        let (module, attr) = app.split_once(":").unwrap_or((app, "app"));
        let (executor, handles) = match WebSocketExecutor::new(
            module,
            attr,
            constants,
            worker_threads,
            enable_lifespan,
        ) {
            Ok(executor) => executor,
            Err(e) => {
                Python::attach(|py| {
                    let tb = e
                        .traceback(py)
                        .and_then(|tb| tb.format().ok())
                        .unwrap_or_default();
                    envoy_log_error!("Failed to initialize ASGI app\n{}{}", tb, e);
                });
                return None;
            }
        };
        Some(Self {
            executor,
            handles: Some(handles),
            max_message_size,
            compression,
        })
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        self.executor.shutdown();
        self.handles.take().unwrap().join();
    }
}

impl<ENF: EnvoyNetworkFilter> NetworkFilterConfig<ENF> for Config {
    fn new_network_filter(&self, _envoy: &mut ENF) -> Box<dyn NetworkFilter<ENF>> {
        Box::new(CatchUnwind::new(Filter {
            executor: self.executor.clone(),
            state: WebSocketState::StartHandshake(HandshakeMachine::start_read(
                EnvoyStream::default(),
            )),
            close_frame: None,
            recv_bridge: EventBridge::new(),
            send_bridge: EventBridge::new(),
            downstream_watermark_level: 0,
            max_message_size: self.max_message_size,
            compression: self.compression,
        }))
    }
}

enum WebSocketState {
    StartHandshake(HandshakeMachine<EnvoyStream>),
    FinishHandshake(EnvoyStream, http::Response<()>, Extensions),
    Accepted(WebSocket<EnvoyStream>),
    NotWebSocket,

    Done, // Can be used as a temporary value during state transitions or to signal the response is done before handshake.
}

struct Filter {
    executor: WebSocketExecutor,

    state: WebSocketState,
    close_frame: Option<CloseFrame>,

    recv_bridge: EventBridge<RecvFuture>,
    send_bridge: EventBridge<SendEvent>,

    downstream_watermark_level: usize,

    max_message_size: usize,
    compression: bool,
}

impl<ENF: EnvoyNetworkFilter> NetworkFilter<ENF> for Filter {
    fn on_read(
        &mut self,
        envoy_filter: &mut ENF,
        _data_length: usize,
        _end_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_network_filter_data_status {
        match &self.state {
            WebSocketState::NotWebSocket => {
                return abi::envoy_dynamic_module_type_on_network_filter_data_status::Continue;
            }
            WebSocketState::Accepted(_) => {
                self.read_web_socket(envoy_filter);
                return abi::envoy_dynamic_module_type_on_network_filter_data_status::StopIteration;
            }
            WebSocketState::FinishHandshake(_, _, _) => {
                // This is similar to junk after handshake request, which we handle above in tail.empty.
                // As we've already executed the app, there isn't much to do but fail the request
                envoy_filter.close_with_details(
                    abi::envoy_dynamic_module_type_network_connection_close_type::AbortReset,
                    "Unexpected body during handshake",
                );
                return abi::envoy_dynamic_module_type_on_network_filter_data_status::StopIteration;
            }
            WebSocketState::Done => {
                // In practice, this shouldn't happen since it means we already closed the connection.
                return abi::envoy_dynamic_module_type_on_network_filter_data_status::StopIteration;
            }
            // The handshake machine is moved while processing the handshake, so we handle it separately
            // from where we don't want to move.
            _ => {}
        };

        if let WebSocketState::StartHandshake(mut mach) =
            std::mem::replace(&mut self.state, WebSocketState::NotWebSocket)
        {
            mach.get_mut().read_from_buffered(envoy_filter);
            loop {
                mach = match mach.single_round::<http::Request<()>>() {
                    Ok(RoundResult::WouldBlock(m)) => {
                        self.state = WebSocketState::StartHandshake(m);
                        return abi::envoy_dynamic_module_type_on_network_filter_data_status::StopIteration;
                    }
                    Ok(RoundResult::Incomplete(m)) => m,
                    Ok(RoundResult::StageFinished(finish)) => {
                        if let StageResult::DoneReading {
                            stream,
                            result: request,
                            tail,
                        } = finish
                        {
                            let Ok(mut response) = create_response(&request) else {
                                // We get here if there's no upgrade header, etc.
                                self.state = WebSocketState::NotWebSocket;
                                return abi::envoy_dynamic_module_type_on_network_filter_data_status::Continue;
                            };
                            if !tail.is_empty() {
                                envoy_filter.close_with_details(
                                    abi::envoy_dynamic_module_type_network_connection_close_type::AbortReset,
                                    "Unexpected body during handshake",
                                );
                                return abi::envoy_dynamic_module_type_on_network_filter_data_status::StopIteration;
                            }
                            // Negotiate permessage-deflate against the client's offers,
                            // writing the agreed extension header into the response. A
                            // malformed extensions header falls back to no compression.
                            let mut extensions_config = ExtensionsConfig::default();
                            if self.compression {
                                extensions_config.permessage_deflate =
                                    Some(DeflateConfig::default());
                            }
                            let extensions = extensions_config
                                .negotiate_response(request.headers(), response.headers_mut())
                                .unwrap_or_default();
                            self.state =
                                WebSocketState::FinishHandshake(stream, response, extensions);
                            let (_, total_size) = envoy_filter.get_read_buffer_chunks();
                            envoy_filter.drain_read_buffer(total_size);
                            let subprotocols = parse_subprotocols(request.headers());
                            let scope = new_scope(request, envoy_filter);
                            self.executor.execute_app(
                                scope,
                                subprotocols,
                                self.recv_bridge.clone(),
                                self.send_bridge.clone(),
                                Box::from(envoy_filter.new_scheduler()),
                            );

                            return abi::envoy_dynamic_module_type_on_network_filter_data_status::StopIteration;
                        } else {
                            // Only reading, not writing handshake.
                            unreachable!()
                        }
                    }
                    Err(_) => {
                        // We get here if the request method wasn't GET.
                        self.state = WebSocketState::NotWebSocket;
                        return abi::envoy_dynamic_module_type_on_network_filter_data_status::Continue;
                    }
                }
            }
        }

        unreachable!()
    }

    fn on_scheduled(&mut self, envoy_filter: &mut ENF, event_id: u64) {
        if event_id == EVENT_ID_REQUEST {
            match &self.state {
                WebSocketState::FinishHandshake(_, _, _) => {
                    let future = self.recv_bridge.get().unwrap();
                    self.executor.handle_recv_future_connect(future);
                }
                WebSocketState::Accepted(_) => {
                    self.read_web_socket(envoy_filter);
                }
                _ => {}
            }
            return;
        }
        if self.downstream_watermark_level == 0 {
            self.process_send_events(envoy_filter);
        }
    }

    fn on_above_write_buffer_high_watermark(&mut self, _envoy_filter: &mut ENF) {
        self.downstream_watermark_level += 1;
    }

    fn on_below_write_buffer_low_watermark(&mut self, envoy_filter: &mut ENF) {
        self.downstream_watermark_level -= 1;
        if self.downstream_watermark_level == 0 {
            envoy_filter.new_scheduler().commit(EVENT_ID_RESPONSE);
        }
    }

    fn on_destroy(&mut self, _envoy_filter: &mut ENF) {
        self.cleanup();
    }
}

impl Filter {
    fn process_send_events(&mut self, envoy_filter: &mut impl EnvoyNetworkFilter) {
        match std::mem::replace(&mut self.state, WebSocketState::Done) {
            WebSocketState::FinishHandshake(stream, mut response, extensions) => {
                let event = self.send_bridge.get().unwrap();
                let event = match event {
                    SendEvent::Close { code, reason } => {
                        let mut response = http::Response::new(());
                        *response.status_mut() = http::StatusCode::FORBIDDEN;
                        let mut buffer = Vec::new();
                        write_response(&mut buffer, &response).unwrap();
                        envoy_filter.write(&buffer, true);
                        self.close_frame = Some(CloseFrame {
                            code: CloseCode::from(code),
                            reason: reason.clone(),
                        });
                        return;
                    }
                    SendEvent::Accept(e) => e,
                    SendEvent::Abort => {
                        envoy_filter.close(
                            abi::envoy_dynamic_module_type_network_connection_close_type::FlushWrite,
                        );
                        return;
                    }
                    SendEvent::Exception => {
                        // Follow uvicorn's behavior which returns 500 for exceptions before handshake.
                        let body = b"Internal Server Error";
                        let mut response = http::Response::new(body);
                        *response.status_mut() = http::StatusCode::INTERNAL_SERVER_ERROR;
                        response
                            .headers_mut()
                            .insert(header::CONTENT_LENGTH, HeaderValue::from_static("21"));
                        let mut buffer = Vec::new();
                        write_response(&mut buffer, &response).unwrap();
                        buffer.extend_from_slice(*response.body());
                        envoy_filter.write(&buffer, true);
                        // Mainly to handle if the application created a task that calls recv, after the app
                        // finished with exception. It's not a meaningful app in the real-world.
                        self.close_frame = Some(CloseFrame {
                            code: CloseCode::Error,
                            reason: Utf8Bytes::default(),
                        });
                        return;
                    }
                    _ => unreachable!(), // SendCallable ensures
                };
                if let Some(subprotocol) = &event.subprotocol
                    && let Ok(value) = HeaderValue::from_bytes(subprotocol.as_bytes())
                {
                    response
                        .headers_mut()
                        .append(header::SEC_WEBSOCKET_PROTOCOL, value);
                }
                for (name, value) in event.headers.iter() {
                    response.headers_mut().append(name, value.clone());
                }
                let mut buffer = Vec::new();
                write_response(&mut buffer, &response).unwrap();
                envoy_filter.write(&buffer, false);
                let ws_config =
                    WebSocketConfig::default().max_message_size(Some(self.max_message_size));
                let websocket = WebSocket::from_raw_socket_with_extensions(
                    stream,
                    Role::Server,
                    Some(ws_config),
                    extensions,
                );
                self.state = WebSocketState::Accepted(websocket);
                // In case there were any recv tasks started before sending the acceptance.
                envoy_filter.new_scheduler().commit(EVENT_ID_REQUEST);
                self.executor.handle_send_future(event.future);
                return;
            }
            other => {
                self.state = other;
            }
        }

        self.send_bridge.process(|event| {
            match &mut self.state {
                WebSocketState::Accepted(websocket) => match event {
                    SendEvent::SendMessage(event) => {
                        if self.close_frame.is_some() {
                            return;
                        }
                        websocket
                            .send(match event.body {
                                Body::Text(text) => Message::Text(text),
                                Body::Bytes(bytes) => Message::Binary(bytes),
                            })
                            .unwrap();
                        websocket.get_mut().write_to(envoy_filter);
                        self.executor.handle_send_future(event.future);
                    }
                    SendEvent::Close { code, reason } => {
                        let close_frame = CloseFrame {
                            code: CloseCode::from(code),
                            reason: reason.clone(),
                        };
                        let _ = websocket.close(Some(close_frame.clone()));
                        websocket.get_mut().write_to(envoy_filter);
                        self.close_frame = Some(close_frame);
                    }
                    SendEvent::Exception => {
                        let close_frame = CloseFrame {
                            code: CloseCode::Error,
                            reason: Utf8Bytes::default(),
                        };
                        let _ = websocket.close(Some(close_frame.clone()));
                        websocket.get_mut().write_to(envoy_filter);
                        self.close_frame = Some(close_frame);
                    }
                    SendEvent::Abort => {
                        // Don't abort if we're already closing gracefully.
                        if self.close_frame.is_none() {
                            envoy_filter.close(
                                abi::envoy_dynamic_module_type_network_connection_close_type::FlushWrite,
                            );
                        }
                    }
                    _ => unreachable!(),
                },
                WebSocketState::Done => {}
                _ => {
                    // Allow to drop to signal closed.
                }
            }
        });
    }

    fn read_web_socket(&mut self, envoy_filter: &mut impl EnvoyNetworkFilter) {
        let WebSocketState::Accepted(websocket) = &mut self.state else {
            unreachable!()
        };

        if let Some(close_frame) = &self.close_frame {
            self.recv_bridge.process(|future| {
                self.executor.handle_recv_future_disconnect(
                    close_frame.code.into(),
                    close_frame.reason.clone(),
                    future,
                );
            });
            finish_close(websocket, envoy_filter);
            self.send_bridge.close();
            self.recv_bridge.close();
        }

        if self.recv_bridge.is_empty() {
            return;
        }

        loop {
            let message = websocket.read();
            websocket.get_mut().write_to(envoy_filter);
            match message {
                Ok(msg) => {
                    let body = match msg {
                        Message::Binary(bytes) => Body::Bytes(bytes),
                        Message::Text(text) => Body::Text(text),
                        Message::Close(frame) => {
                            let close_frame = frame.unwrap_or(CloseFrame {
                                code: CloseCode::Status,
                                reason: Utf8Bytes::default(),
                            });
                            self.close_frame = Some(close_frame.clone());
                            finish_close(websocket, envoy_filter);
                            self.recv_bridge.process(|future| {
                                self.executor.handle_recv_future_disconnect(
                                    close_frame.code.into(),
                                    close_frame.reason.clone(),
                                    future,
                                );
                            });
                            self.cleanup();
                            return;
                        }
                        _ => continue,
                    };
                    if let Some(future) = self.recv_bridge.get() {
                        self.executor.handle_recv_future_message(body, future);
                    }
                }
                Err(tungstenite::Error::Io(ref e))
                    if e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    // Need more bytes to parse a frame. Pull the next block from
                    // Envoy if available, or wait otherwise.
                    if websocket.get_mut().read_block(envoy_filter) == 0 {
                        break;
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
                    self.close_frame = Some(close_frame.clone());
                    close(
                        close_frame,
                        &self.executor,
                        &mut self.recv_bridge,
                        websocket,
                        envoy_filter,
                    );
                    self.cleanup();
                    return;
                }
            }
            if self.recv_bridge.is_empty() {
                break;
            }
        }
    }

    fn cleanup(&self) {
        self.recv_bridge.close();
        self.send_bridge.close();
    }
}

fn close(
    close_frame: CloseFrame,
    executor: &WebSocketExecutor,
    recv_bridge: &mut EventBridge<RecvFuture>,
    websocket: &mut WebSocket<EnvoyStream>,
    envoy_filter: &mut impl EnvoyNetworkFilter,
) {
    let _ = websocket.close(Some(close_frame.clone()));
    finish_close(websocket, envoy_filter);
    recv_bridge.process(|future| {
        executor.handle_recv_future_disconnect(
            close_frame.code.into(),
            close_frame.reason.clone(),
            future,
        );
    });
}

fn finish_close(
    websocket: &mut WebSocket<EnvoyStream>,
    envoy_filter: &mut impl EnvoyNetworkFilter,
) {
    // Closing a websocket involves exchanging messages. This may cross network events,
    // so we read as much and write as much as we can. Eventually it will be done.
    websocket.get_mut().read_from(envoy_filter);
    loop {
        let message = websocket.read();
        websocket.get_mut().write_to(envoy_filter);
        // We shouldn't see any more real messages but it's fine to ignore them.
        // Just check for when we need more bytes.
        match message {
            Err(tungstenite::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                break;
            }
            Err(tungstenite::Error::AlreadyClosed) => {
                break;
            }
            Err(tungstenite::Error::ConnectionClosed) => {
                envoy_filter.close(
                    abi::envoy_dynamic_module_type_network_connection_close_type::FlushWrite,
                );
                break;
            }
            Err(_) => {
                envoy_filter.close(
                    abi::envoy_dynamic_module_type_network_connection_close_type::AbortReset,
                );
                break;
            }
            _ => {}
        }
    }
}

/// Parses the comma-separated subprotocols offered by the client across any
/// number of `Sec-WebSocket-Protocol` headers, in offer order.
fn parse_subprotocols(headers: &http::HeaderMap) -> Vec<String> {
    headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|protocol| !protocol.is_empty())
        .map(str::to_owned)
        .collect()
}

fn new_scope(request: http::Request<()>, envoy_filter: &mut impl EnvoyNetworkFilter) -> Scope {
    let (head, _) = request.into_parts();
    let mut headers: Vec<(HeaderName, HeaderValue)> = Vec::with_capacity(head.headers.len());
    let mut current_name: Option<HeaderName> = None;
    for (name, value) in head.headers.into_iter() {
        if let Some(name) = name {
            current_name = Some(name);
        }
        headers.push((current_name.as_ref().unwrap().clone(), value));
    }

    let is_ssl = envoy_filter.is_ssl();

    Scope {
        http_version: head.version,
        method: head.method,
        scheme: if is_ssl {
            uri::Scheme::try_from("wss").unwrap()
        } else {
            uri::Scheme::try_from("ws").unwrap()
        },
        raw_path: head
            .uri
            .path_and_query()
            .map(|pq| Box::from(pq.as_str().as_bytes()))
            .unwrap_or_default(),
        headers,
        client: address_to_scope(envoy_filter.get_remote_address()),
        server: address_to_scope(envoy_filter.get_local_address()),
        // Network filter doesn't expose tls_version but we can report the rest.
        tls_info: is_ssl.then(|| TlsInfo {
            tls_version: None,
            client_cert_name: envoy_filter
                .get_ssl_subject()
                .map(|s| Box::from(String::from_utf8_lossy(s.as_slice()))),
        }),
    }
}

/// Converts an Envoy `(address, port)` pair into an ASGI scope address, dropping
/// any port suffix on the host and treating an empty host as "no address".
fn address_to_scope((address, port): (String, u32)) -> Option<(Box<str>, i64)> {
    if address.is_empty() {
        return None;
    }
    Some((Box::from(strip_port(&address)), i64::from(port)))
}
