use envoy_proxy_dynamic_modules_rust_sdk::*;
use http::{HeaderName, HeaderValue, header};
use pyo3::Python;
use pyo3::types::PyTracebackMethods as _;
use std::sync::Arc;
use tungstenite::handshake::machine::{HandshakeMachine, RoundResult, StageResult};
use tungstenite::handshake::server::{create_response, write_response};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role};
use tungstenite::{Message, Utf8Bytes, WebSocket};

use crate::asgi::python::EVENT_ID_REQUEST;
use crate::asgi::shared::ExecutorHandles;
use crate::asgi::websocket::executor::{Body, RecvFuture, SendEvent, WebSocketExecutor};
use crate::asgi::websocket::stream::EnvoyStream;
use crate::eventbridge::EventBridge;
use crate::types::*;

mod executor;
mod stream;

pub struct Config {
    executor: WebSocketExecutor,
    handles: Option<ExecutorHandles>,
}

impl Config {
    pub fn new(
        app: &str,
        constants: Arc<Constants>,
        worker_threads: usize,
        enable_lifespan: Option<bool>,
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
        Box::new(Filter {
            executor: self.executor.clone(),
            state: WebSocketState::StartHandshake(HandshakeMachine::start_read(
                EnvoyStream::default(),
            )),
            close_frame: None,
            recv_bridge: EventBridge::new(),
            send_bridge: EventBridge::new(),
        })
    }
}

enum WebSocketState {
    StartHandshake(HandshakeMachine<EnvoyStream>),
    FinishHandshake(EnvoyStream, http::Response<()>),
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
}

impl<ENF: EnvoyNetworkFilter> NetworkFilter<ENF> for Filter {
    fn on_destroy(&mut self, _envoy_filter: &mut ENF) {
        self.recv_bridge.close();
        self.send_bridge.close();
    }

    fn on_read(
        &mut self,
        envoy_filter: &mut ENF,
        data_length: usize,
        end_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_network_filter_data_status {
        match &self.state {
            WebSocketState::NotWebSocket => {
                eprintln!("on_read: NotWebSocket");
                return abi::envoy_dynamic_module_type_on_network_filter_data_status::Continue;
            }
            WebSocketState::Accepted(_) => {
                eprintln!("on_read: Accepted");
                self.read_web_socket(envoy_filter);
                return abi::envoy_dynamic_module_type_on_network_filter_data_status::StopIteration;
            }
            WebSocketState::FinishHandshake(_, _) => {
                eprintln!("on_read: FinishHandshake");
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
            eprintln!("on_read: StartHandshake");
            eprintln!(
                "StartHandshake: data_length={}, end_stream={}",
                data_length, end_stream
            );
            mach.get_mut().read_from_buffered(envoy_filter);
            loop {
                eprintln!("HandshakeMachine round");
                mach = match mach.single_round::<http::Request<()>>() {
                    Ok(RoundResult::WouldBlock(m)) => {
                        eprintln!("HandshakeMachine would block");
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
                            let Ok(response) = create_response(&request) else {
                                // We get here if there's no upgrade header, etc.
                                self.state = WebSocketState::NotWebSocket;
                                return abi::envoy_dynamic_module_type_on_network_filter_data_status::Continue;
                            };
                            eprintln!("HandshakeMachine finished: tail_len={}", tail.len());
                            if !tail.is_empty() {
                                envoy_filter.close_with_details(
                                    abi::envoy_dynamic_module_type_network_connection_close_type::AbortReset,
                                    "Unexpected body during handshake",
                                );
                                return abi::envoy_dynamic_module_type_on_network_filter_data_status::StopIteration;
                            }
                            self.state = WebSocketState::FinishHandshake(stream, response);
                            let (_, total_size) = envoy_filter.get_read_buffer_chunks();
                            envoy_filter.drain_read_buffer(total_size);
                            let scope = new_scope(request, envoy_filter);
                            self.executor.execute_app(
                                scope,
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
        eprintln!("on_scheduled: event_id={}", event_id);
        if event_id == EVENT_ID_REQUEST {
            match &self.state {
                WebSocketState::FinishHandshake(_, _) => {
                    eprintln!("send websocket.connect");
                    let future = self.recv_bridge.get().unwrap();
                    self.executor.handle_recv_future_connect(future);
                }
                WebSocketState::Accepted(_) => {
                    eprintln!("read websocket");
                    self.read_web_socket(envoy_filter);
                }
                _ => {}
            }
            return;
        }
        self.process_send_events(envoy_filter);
    }
}

impl Filter {
    fn process_send_events(&mut self, envoy_filter: &mut impl EnvoyNetworkFilter) {
        match std::mem::replace(&mut self.state, WebSocketState::Done) {
            WebSocketState::FinishHandshake(stream, mut response) => {
                eprintln!("processing send events: FinishHandshake");
                let event = self.send_bridge.get().unwrap();
                let event = match event {
                    SendEvent::Close { code, reason } => {
                        eprintln!("Sending close during handshake");
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
                    SendEvent::Exception => {
                        eprintln!("Exception during handshake");
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
                for (name, value) in event.headers.iter() {
                    response.headers_mut().append(name, value.clone());
                }
                eprintln!("Sending handshake response");
                let mut buffer = Vec::new();
                write_response(&mut buffer, &response).unwrap();
                envoy_filter.write(&buffer, false);
                let websocket = WebSocket::from_raw_socket(stream, Role::Server, None);
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

        eprintln!("processing send events: Accepted");
        self.send_bridge.process(|event| {
            eprintln!("processing send event");
            match &mut self.state {
                WebSocketState::Accepted(websocket) => match event {
                    SendEvent::SendMessage(event) => {
                        if self.close_frame.is_some() {
                            eprintln!("ignoring send message after close");
                            return;
                        }
                        eprintln!("Sending websocket message");
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
                        eprintln!("Sending close message");
                        let close_frame = CloseFrame {
                            code: CloseCode::from(code),
                            reason: reason.clone(),
                        };
                        let _ = websocket.close(Some(close_frame.clone()));
                        websocket.get_mut().write_to(envoy_filter);
                        self.close_frame = Some(close_frame);
                    }
                    SendEvent::Exception => {
                        eprintln!("exception");
                        let close_frame = CloseFrame {
                            code: CloseCode::Error,
                            reason: Utf8Bytes::default(),
                        };
                        let _ = websocket.close(Some(close_frame.clone()));
                        websocket.get_mut().write_to(envoy_filter);
                        self.close_frame = Some(close_frame);
                    }
                    _ => unreachable!(),
                },
                WebSocketState::Done => {
                    eprintln!("Sentinel");
                }
                _ => {
                    eprintln!("Other");
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
            eprintln!("handling already closed");
            self.recv_bridge.process(|future| {
                self.executor.handle_recv_future_disconnect(
                    close_frame.code.into(),
                    close_frame.reason.clone(),
                    future,
                );
            });
            finish_close(websocket, envoy_filter);
        }

        if self.recv_bridge.is_empty() {
            eprintln!("recv bridge empty");
            return;
        }

        eprintln!("recv bridge not empty");
        // TODO: We should read in blocks until the first message is ready for backpressure but keep
        // simple for now.
        websocket.get_mut().read_from(envoy_filter);
        loop {
            let message = websocket.read();
            websocket.get_mut().write_to(envoy_filter);
            match message {
                Ok(msg) => {
                    eprintln!("Reading message");
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
                            return;
                        }
                        _ => continue,
                    };
                    eprintln!("Sending read message");
                    if let Some(future) = self.recv_bridge.get() {
                        self.executor.handle_recv_future_message(body, future);
                    }
                }
                Err(tungstenite::Error::Io(ref e))
                    if e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    eprintln!("WouldBlock");
                    break;
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
                    return;
                }
            }
            if self.recv_bridge.is_empty() {
                break;
            }
        }
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
    eprintln!("finish_close");
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

    Scope {
        http_version: head.version,
        method: head.method,
        scheme: head
            .uri
            .scheme()
            .cloned()
            .unwrap_or(http::uri::Scheme::HTTP),
        raw_path: head
            .uri
            .path_and_query()
            .map(|pq| Box::from(pq.as_str().as_bytes()))
            .unwrap_or_default(),
        headers,
        // TODO
        client: None,
        server: None,
        tls_info: None,
    }
}
