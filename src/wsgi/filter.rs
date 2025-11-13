use crate::envoy::{SyncScheduler, has_request_body};
use crate::eventbridge::EventBridge;
use crate::wsgi::python::Executor;
use envoy_proxy_dynamic_modules_rust_sdk::*;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, mpsc};

use super::types::*;
use crate::types::*;
pub struct Config {
    executor: Executor,
}

impl Config {
    pub fn new(app: &str, constants: Arc<Constants>) -> Option<Self> {
        let (module, attr) = app.split_once(":").unwrap_or((app, "app"));
        let executor = match Executor::new(module, attr, 200, constants) {
            Ok(executor) => executor,
            Err(err) => {
                eprintln!("Failed to initialize WSGI app: {err}");
                return None;
            }
        };
        Some(Self { executor })
    }
}

impl<EHF: EnvoyHttpFilter> HttpFilterConfig<EHF> for Config {
    fn new_http_filter(&mut self, _envoy: &mut EHF) -> Box<dyn HttpFilter<EHF>> {
        let (request_body_tx, request_body_rx) = mpsc::channel::<RequestBody>();
        let (response_written_tx, response_written_rx) = mpsc::channel::<()>();
        Box::new(Filter {
            executor: self.executor.clone(),
            request_closed: false,
            response_closed: false,
            request_read_bridge: EventBridge::new(),
            response_bridge: EventBridge::new(),
            request_body_rx: Some(request_body_rx),
            request_body_tx,
            pending_read: RequestReadEvent::Wait,
            read_buffer: Vec::new(),
            response_written_tx,
            response_written_rx: Some(response_written_rx),
            start_event: None,
        })
    }
}

struct Filter {
    executor: Executor,

    request_closed: bool,
    response_closed: bool,

    /// The start event sent by the application. In ASGI, we buffer this within the ASGI
    /// implementation itself to reduce filter wakeups, but because of the
    /// WSGI write function allowing the body to be sent from two different locations,
    /// it ends up far easier to buffer it in the filter for WSGI.
    start_event: Option<ResponseStartEvent>,

    request_read_bridge: EventBridge<RequestReadEvent>,
    response_bridge: EventBridge<ResponseEvent>,
    pending_read: RequestReadEvent,
    read_buffer: Vec<u8>,

    request_body_rx: Option<Receiver<RequestBody>>,
    request_body_tx: Sender<RequestBody>,

    response_written_tx: Sender<()>,
    response_written_rx: Option<Receiver<()>>,
}

impl<EHF: EnvoyHttpFilter> HttpFilter<EHF> for Filter {
    fn on_request_headers(
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_headers_status {
        let scope = new_scope(envoy_filter);

        self.executor.execute_app(
            scope,
            self.request_read_bridge.clone(),
            self.request_body_rx.take().unwrap(),
            self.response_bridge.clone(),
            self.response_written_rx.take().unwrap(),
            SyncScheduler::new(envoy_filter.new_scheduler()),
        );
        if end_of_stream {
            self.request_closed = true;
            abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::ContinueAndDontEndStream
        } else {
            abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::Continue
        }
    }

    fn on_request_body(
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_body_status {
        if end_of_stream {
            self.request_closed = true;
        }
        self.process_read(envoy_filter);
        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::StopIterationAndBuffer
    }

    fn on_stream_complete(&mut self, _envoy_filter: &mut EHF) {
        self.response_closed = true;
        self.request_read_bridge.close();
        self.response_bridge.close();
    }

    fn on_scheduled(&mut self, envoy_filter: &mut EHF, event_id: u64) {
        if event_id == EVENT_ID_REQUEST {
            let mut processed = false;
            self.request_read_bridge.process(|event| {
                self.pending_read = event;
                processed = true;
            });
            if processed {
                self.process_read(envoy_filter);
            }
            return;
        }
        self.response_bridge.process(|event| match event {
            ResponseEvent::Start(start_event) => {
                self.start_event.replace(start_event);
            }
            ResponseEvent::Body(body_event) => {
                if let Some(start_event) = self.start_event.take() {
                    let mut status_buf = itoa::Buffer::new();
                    let mut headers: Vec<(&str, &[u8])> =
                        Vec::with_capacity(start_event.headers.len() + 1);
                    headers.push((":status", status_buf.format(start_event.status).as_bytes()));
                    for (k, v) in start_event.headers.iter() {
                        headers.push((k.as_str(), v.as_bytes()));
                    }
                    let end_stream = !body_event.more_body;
                    if !end_stream {
                        send_or_log(&self.response_written_tx, ());
                    }
                    if end_stream {
                        if body_event.body.is_empty() {
                            envoy_filter.send_response_headers(headers, true);
                        } else {
                            envoy_filter.send_response_headers(headers, false);
                            envoy_filter.send_response_data(&body_event.body, true);
                        }
                    } else {
                        envoy_filter.send_response_headers(headers, false);
                        envoy_filter.send_response_data(&body_event.body, false);
                    }
                } else {
                    let end_stream = !body_event.more_body;
                    if !end_stream {
                        send_or_log(&self.response_written_tx, ());
                    }
                    envoy_filter.send_response_data(&body_event.body, end_stream);
                }
            }
            ResponseEvent::Exception => {
                if !self.response_closed {
                    self.response_closed = true;
                    envoy_filter.send_response(
                        500,
                        vec![
                            ("content-type", b"text/plain; charset=utf-8"),
                            ("connection", b"close"),
                        ],
                        Some(b"Internal Server Error"),
                    );
                }
            }
        });
    }
}

impl Filter {
    fn process_read<EHF: EnvoyHttpFilter>(&mut self, envoy_filter: &mut EHF) {
        // Reads always block until there is data or the request is finished,
        // so don't process them otherwise.
        if !has_request_body(envoy_filter) && !self.request_closed {
            return;
        }
        match self.pending_read {
            RequestReadEvent::Raw(n) if n > 0 => {
                let mut remaining = n as usize;
                let mut body: Vec<u8> = Vec::with_capacity(remaining);
                if let Some(buffers) = envoy_filter.get_request_body() {
                    for buffer in buffers {
                        let to_read = std::cmp::min(remaining, buffer.as_slice().len());
                        body.extend_from_slice(&buffer.as_slice()[..to_read]);
                        remaining -= to_read;
                        if remaining == 0 {
                            break;
                        }
                    }
                    envoy_filter.drain_request_body(body.len());
                }
                self.pending_read = RequestReadEvent::Wait;
                send_or_log(
                    &self.request_body_tx,
                    RequestBody {
                        body: body.into_boxed_slice(),
                        closed: !has_request_body(envoy_filter) && self.request_closed,
                    },
                );
            }
            RequestReadEvent::Raw(_) => {
                if let Some(buffers) = envoy_filter.get_request_body() {
                    let mut read = 0;
                    for buffer in buffers {
                        self.read_buffer.extend_from_slice(buffer.as_slice());
                        read += buffer.as_slice().len();
                    }
                    envoy_filter.drain_request_body(read);
                }
                if self.request_closed {
                    self.pending_read = RequestReadEvent::Wait;
                    send_or_log(
                        &self.request_body_tx,
                        RequestBody {
                            body: std::mem::take(&mut self.read_buffer).into_boxed_slice(),
                            closed: true,
                        },
                    );
                }
            }
            RequestReadEvent::Line(n) => {
                if let Some(buffers) = envoy_filter.get_request_body() {
                    let mut read = 0;
                    let mut send = false;
                    'outer: for buffer in buffers {
                        for &b in buffer.as_slice() {
                            self.read_buffer.push(b);
                            read += 1;
                            if b == b'\n' || (n > 0 && self.read_buffer.len() >= n as usize) {
                                send = true;
                                break 'outer;
                            }
                        }
                    }
                    envoy_filter.drain_request_body(read);
                    if send || self.request_closed {
                        let body = std::mem::take(&mut self.read_buffer);
                        self.pending_read = RequestReadEvent::Wait;
                        send_or_log(
                            &self.request_body_tx,
                            RequestBody {
                                body: body.into_boxed_slice(),
                                closed: !has_request_body(envoy_filter) && self.request_closed,
                            },
                        );
                    }
                }
            }
            _ => (),
        }
    }
}

fn send_or_log<T>(tx: &Sender<T>, value: T) {
    if let Err(err) = tx.send(value) {
        eprintln!(
            "Failed to send event to Python, this is likely a bug in pyvoy: {}",
            err,
        );
    }
}
