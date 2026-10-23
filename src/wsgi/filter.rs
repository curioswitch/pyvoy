use crate::envoy::{buffers_len, extend_from_buffers, has_request_body};
use crate::eventbridge::EventBridge;
use crate::wsgi::python::Executor;
use crate::wsgi::transport::{
    ResetReason, SendStreamDataEvent, TransportEvent, TransportResponse, TransportState,
};
use ::http::{HeaderName, HeaderValue, StatusCode};
use envoy_proxy_dynamic_modules_rust_sdk::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, mpsc};

use super::types::*;
use crate::types::*;
pub struct Config {
    executor: Executor,
    enable_http_client: bool,
}

impl Config {
    pub fn new(
        app: &str,
        root_path: &str,
        constants: Arc<Constants>,
        worker_threads: usize,
        enable_http_client: bool,
    ) -> Option<Self> {
        let (module, attr) = app.split_once(":").unwrap_or((app, "app"));
        let executor = match Executor::new(module, attr, root_path, worker_threads, constants) {
            Ok(executor) => executor,
            Err(err) => {
                eprintln!("Failed to initialize WSGI app: {err}");
                return None;
            }
        };
        Some(Self {
            executor,
            enable_http_client,
        })
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        self.executor.shutdown();
    }
}

impl<EHF: EnvoyHttpFilter> HttpFilterConfig<EHF> for Config {
    fn new_http_filter(&self, _envoy: &mut EHF) -> Box<dyn HttpFilter<EHF>> {
        let (request_body_tx, request_body_rx) = mpsc::channel::<RequestBody>();
        let (response_written_tx, response_written_rx) = mpsc::channel::<()>();
        Box::new(CatchUnwind::new(Filter {
            executor: self.executor.clone(),
            request_closed: Cell::new(false),
            response_closed: Cell::new(false),
            request_read_bridge: EventBridge::new(),
            response_bridge: EventBridge::new(),
            transport_bridge: self.enable_http_client.then(EventBridge::new),
            transport_responses: RefCell::new(HashMap::new()),
            request_body_rx: Cell::new(Some(request_body_rx)),
            request_body_tx,
            pending_read: Cell::new(RequestReadEvent::Wait),
            read_buffer: RefCell::new(Vec::new()),
            response_written_tx,
            response_written_rx: Cell::new(Some(response_written_rx)),
            downstream_watermark_level: Cell::new(0),
        }))
    }
}

struct Filter {
    executor: Executor,

    request_closed: Cell<bool>,
    response_closed: Cell<bool>,

    request_read_bridge: EventBridge<RequestReadEvent>,
    response_bridge: EventBridge<ResponseEvent>,
    pending_read: Cell<RequestReadEvent>,
    read_buffer: RefCell<Vec<u8>>,

    /// Only present when the HTTP client is enabled.
    transport_bridge: Option<EventBridge<TransportEvent>>,
    transport_responses: RefCell<HashMap<u64, TransportState>>,

    /// Handed to the executor in `on_request_headers`.
    request_body_rx: Cell<Option<Receiver<RequestBody>>>,
    request_body_tx: Sender<RequestBody>,

    response_written_tx: Sender<()>,
    /// Handed to the executor in `on_request_headers`.
    response_written_rx: Cell<Option<Receiver<()>>>,

    downstream_watermark_level: Cell<usize>,
}

impl<EHF: EnvoyHttpFilter> HttpFilter<EHF> for Filter {
    fn on_request_headers(
        &self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_headers_status {
        let scope = new_scope(envoy_filter);

        self.executor.execute_app(
            scope,
            end_of_stream,
            self.request_read_bridge.clone(),
            self.request_body_rx.take().unwrap(),
            self.response_bridge.clone(),
            self.response_written_rx.take().unwrap(),
            self.transport_bridge.clone(),
            Box::from(envoy_filter.new_scheduler()),
        );
        if end_of_stream {
            self.request_closed.set(true);
        }
        abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::StopIteration
    }

    fn on_request_body(
        &self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_body_status {
        if end_of_stream {
            self.request_closed.set(true);
        }

        self.process_read(envoy_filter);

        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::StopIterationAndWatermark
    }

    fn on_request_trailers(
        &self,
        envoy_filter: &mut EHF,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_trailers_status {
        self.request_closed.set(true);

        self.process_read(envoy_filter);

        abi::envoy_dynamic_module_type_on_http_filter_request_trailers_status::StopIteration
    }

    fn on_stream_complete(&self, _envoy_filter: &mut EHF) {
        self.response_closed.set(true);
        self.request_read_bridge.close();
        self.response_bridge.close();
        if let Some(transport_bridge) = &self.transport_bridge {
            transport_bridge.close();
        }

        let transport_responses: Vec<TransportState> = self
            .transport_responses
            .borrow_mut()
            .drain()
            .map(|(_, state)| state)
            .collect();
        for state in transport_responses {
            let _ = state.response_tx.send(TransportResponse::Reset {
                reason: ResetReason::ServerRequestDone,
                exception: state.exception,
            });
        }
    }

    fn on_scheduled(&self, envoy_filter: &mut EHF, event_id: u64) {
        match event_id {
            EVENT_ID_REQUEST => self.process_read(envoy_filter),
            EVENT_ID_RESPONSE => {
                if self.downstream_watermark_level.get() == 0 {
                    self.process_send_events(envoy_filter);
                }
            }
            EVENT_ID_OUTGOING_REQUEST => self.process_transport_events(envoy_filter),
            _ => unreachable!(),
        }
    }

    fn on_downstream_above_write_buffer_high_watermark(&self, _envoy_filter: &mut EHF) {
        self.downstream_watermark_level
            .set(self.downstream_watermark_level.get() + 1);
    }

    fn on_downstream_below_write_buffer_low_watermark(&self, envoy_filter: &mut EHF) {
        let level = self.downstream_watermark_level.get() - 1;
        self.downstream_watermark_level.set(level);
        if level == 0 {
            envoy_filter.new_scheduler().commit(EVENT_ID_RESPONSE);
        }
    }

    fn on_http_stream_headers(
        &self,
        envoy_filter: &mut EHF,
        stream_handle: u64,
        response_headers: &[(EnvoyBuffer, EnvoyBuffer)],
        end_stream: bool,
    ) {
        // Clone the sender out so the map borrow ends before the reset below,
        // which re-enters on_http_stream_reset.
        let response_tx = self.response_tx_for(stream_handle);
        let Some(response_tx) = response_tx else {
            // Happens if there is a configuration bug for the upstream.
            unsafe { envoy_filter.reset_http_stream(stream_handle) };
            return;
        };
        let mut headers: Vec<(HeaderName, HeaderValue)> =
            Vec::with_capacity(response_headers.len());
        let mut status = StatusCode::OK;
        for (k, v) in response_headers {
            if k.as_slice() == b":status" {
                status = StatusCode::from_bytes(v.as_slice())
                    // Should be validated by Envoy already so just fallback.
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                continue;
            }
            match (
                HeaderName::from_bytes(k.as_slice()),
                HeaderValue::from_bytes(v.as_slice()),
            ) {
                (Ok(name), Ok(value)) => headers.push((name, value)),
                // Should be validated by Envoy already so shouldn't happen in practice.
                _ => continue,
            }
        }
        let _ = response_tx.send(TransportResponse::Headers {
            status,
            headers,
            end_stream,
        });
    }

    fn on_http_stream_data(
        &self,
        _envoy_filter: &mut EHF,
        stream_handle: u64,
        response_data: &[EnvoyBuffer],
        end_stream: bool,
    ) {
        let Some(response_tx) = self.response_tx_for(stream_handle) else {
            return;
        };
        let len = response_data.iter().map(|b| b.as_slice().len()).sum();
        let mut body = Vec::with_capacity(len);
        for buffer in response_data {
            body.extend_from_slice(buffer.as_slice());
        }
        if !body.is_empty() {
            let _ = response_tx.send(TransportResponse::Body(body.into_boxed_slice()));
        }
        if end_stream {
            let _ = response_tx.send(TransportResponse::End);
        }
    }

    fn on_http_stream_trailers(
        &self,
        _envoy_filter: &mut EHF,
        stream_handle: u64,
        response_trailers: &[(EnvoyBuffer, EnvoyBuffer)],
    ) {
        let Some(response_tx) = self.response_tx_for(stream_handle) else {
            return;
        };
        let mut trailers: Vec<(HeaderName, HeaderValue)> =
            Vec::with_capacity(response_trailers.len());
        for (name, value) in response_trailers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_slice()),
                HeaderValue::from_bytes(value.as_slice()),
            ) {
                trailers.push((name, value));
            }
        }
        let _ = response_tx.send(TransportResponse::Trailers(trailers));
    }

    fn on_http_stream_reset(
        &self,
        _envoy_filter: &mut EHF,
        stream_handle: u64,
        reason: abi::envoy_dynamic_module_type_http_stream_reset_reason,
    ) {
        let state = self.transport_responses.borrow_mut().remove(&stream_handle);
        let Some(mut state) = state else {
            return;
        };
        let reason = match reason {
            abi::envoy_dynamic_module_type_http_stream_reset_reason::LocalReset
            | abi::envoy_dynamic_module_type_http_stream_reset_reason::LocalRefusedStreamReset => {
                ResetReason::Local
            }
            abi::envoy_dynamic_module_type_http_stream_reset_reason::RemoteReset
            | abi::envoy_dynamic_module_type_http_stream_reset_reason::RemoteRefusedStreamReset => {
                ResetReason::Remote
            }
            abi::envoy_dynamic_module_type_http_stream_reset_reason::ProtocolError => {
                ResetReason::Protocol
            }
            abi::envoy_dynamic_module_type_http_stream_reset_reason::ConnectionFailure
            | abi::envoy_dynamic_module_type_http_stream_reset_reason::ConnectionTermination => {
                ResetReason::Connection
            }
            abi::envoy_dynamic_module_type_http_stream_reset_reason::Overflow => {
                ResetReason::Overflow
            }
        };
        let _ = state.response_tx.send(TransportResponse::Reset {
            reason,
            exception: state.exception.take(),
        });
    }

    fn on_http_stream_complete(&self, _envoy_filter: &mut EHF, stream_handle: u64) {
        // Cleanup for a successful response. It may be more precise to do this on data with
        // end_stream or trailers but simpler to just take care of it here.
        self.transport_responses.borrow_mut().remove(&stream_handle);
    }
}

impl Filter {
    /// The response channel of a live upstream stream, cloned out so the map
    /// borrow ends before the caller does anything else.
    fn response_tx_for(&self, stream_handle: u64) -> Option<Sender<TransportResponse>> {
        self.transport_responses
            .borrow()
            .get(&stream_handle)
            .map(|state| state.response_tx.clone())
    }

    fn process_read<EHF: EnvoyHttpFilter>(&self, envoy_filter: &mut EHF) {
        self.request_read_bridge.process(|event| {
            self.pending_read.set(event);
        });
        // Reads always block until there is data or the request is finished,
        // so don't process them otherwise.
        if !has_request_body(envoy_filter) && !self.request_closed.get() {
            return;
        }
        let pending_read = self.pending_read.get();
        match pending_read {
            RequestReadEvent::Raw(n) if n > 0 => {
                let mut remaining = n as usize;
                let mut body: Vec<u8> = Vec::with_capacity(remaining);
                if let Some(buffers) = envoy_filter.get_buffered_request_body() {
                    let mut read = 0;
                    for buffer in buffers.iter().map(|b| b.as_slice()) {
                        let to_read = std::cmp::min(remaining, buffer.len());
                        body.extend_from_slice(&buffer[..to_read]);
                        remaining -= to_read;
                        read += to_read;
                        if remaining == 0 {
                            break;
                        }
                    }
                    envoy_filter.drain_buffered_request_body(read);
                }
                if remaining > 0
                    && let Some(buffers) = envoy_filter.get_received_request_body()
                {
                    let mut read = 0;
                    for buffer in buffers.iter().map(|b| b.as_slice()) {
                        let to_read = std::cmp::min(remaining, buffer.len());
                        body.extend_from_slice(&buffer[..to_read]);
                        remaining -= to_read;
                        read += to_read;
                        if remaining == 0 {
                            break;
                        }
                    }
                    envoy_filter.drain_received_request_body(read);
                }
                self.pending_read.set(RequestReadEvent::Wait);
                send_or_log(
                    &self.request_body_tx,
                    RequestBody {
                        body: body.into_boxed_slice(),
                        closed: !has_request_body(envoy_filter) && self.request_closed.get(),
                    },
                );
            }
            RequestReadEvent::Raw(_) => {
                let available = buffers_len(&envoy_filter.get_buffered_request_body())
                    + buffers_len(&envoy_filter.get_received_request_body());
                self.read_buffer.borrow_mut().reserve(available);
                let buffered_read = extend_from_buffers(
                    &envoy_filter.get_buffered_request_body(),
                    &mut self.read_buffer.borrow_mut(),
                );
                if buffered_read > 0 {
                    envoy_filter.drain_buffered_request_body(buffered_read);
                }
                let received_read = extend_from_buffers(
                    &envoy_filter.get_received_request_body(),
                    &mut self.read_buffer.borrow_mut(),
                );
                if received_read > 0 {
                    envoy_filter.drain_received_request_body(received_read);
                }
                if self.request_closed.get() {
                    self.pending_read.set(RequestReadEvent::Wait);
                    send_or_log(
                        &self.request_body_tx,
                        RequestBody {
                            body: self.read_buffer.take().into_boxed_slice(),
                            closed: true,
                        },
                    );
                }
            }
            RequestReadEvent::Line(n) => {
                let mut send = false;
                if let Some(buffers) = envoy_filter.get_buffered_request_body() {
                    let (should_send, read) = self.read_request_until_line_or_size(&buffers, n);
                    send = should_send;
                    envoy_filter.drain_buffered_request_body(read);
                }
                if !send && let Some(buffers) = envoy_filter.get_received_request_body() {
                    let (should_send, read) = self.read_request_until_line_or_size(&buffers, n);
                    send = should_send;
                    envoy_filter.drain_received_request_body(read);
                }
                if send || self.request_closed.get() {
                    let body = self.read_buffer.take();
                    self.pending_read.set(RequestReadEvent::Wait);
                    send_or_log(
                        &self.request_body_tx,
                        RequestBody {
                            body: body.into_boxed_slice(),
                            closed: !has_request_body(envoy_filter) && self.request_closed.get(),
                        },
                    );
                }
            }
            _ => (),
        }
    }

    fn read_request_until_line_or_size(
        &self,
        buffers: &Vec<EnvoyMutBuffer<'_>>,
        n: isize,
    ) -> (bool, usize) {
        let mut read_buffer = self.read_buffer.borrow_mut();
        let mut read = 0;
        for buffer in buffers.iter().map(|b| b.as_slice()) {
            for &b in buffer {
                read_buffer.push(b);
                read += 1;
                if b == b'\n' || (n > 0 && read_buffer.len() >= n as usize) {
                    return (true, read);
                }
            }
        }
        (false, read)
    }

    fn process_send_events<EHF: EnvoyHttpFilter>(&self, envoy_filter: &mut EHF) {
        self.response_bridge.process(|event| match event {
            ResponseEvent::Start(start_event, body_event) => {
                let mut headers: Vec<(&str, &[u8])> =
                    Vec::with_capacity(start_event.headers.len() + 1);
                headers.push((":status", start_event.status.as_str().as_bytes()));
                for (k, v) in start_event.headers.iter() {
                    headers.push((k.as_str(), v.as_bytes()));
                }
                let end_stream = !body_event.more_body;
                if !end_stream {
                    send_or_log(&self.response_written_tx, ());
                }
                if end_stream {
                    if body_event.body.is_empty() {
                        envoy_filter.send_response_headers(&headers, true);
                    } else {
                        envoy_filter.send_response_headers(&headers, false);
                        envoy_filter.send_response_data(&body_event.body, true);
                    }
                } else {
                    envoy_filter.send_response_headers(&headers, false);
                    envoy_filter.send_response_data(&body_event.body, false);
                }
            }
            ResponseEvent::Body(body_event) => {
                let end_stream = !body_event.more_body;
                if !end_stream {
                    send_or_log(&self.response_written_tx, ());
                }
                envoy_filter.send_response_data(&body_event.body, end_stream);
            }
            ResponseEvent::Trailers(start_event, trailers) => {
                if let Some(start_event) = start_event {
                    let mut headers: Vec<(&str, &[u8])> =
                        Vec::with_capacity(start_event.headers.len() + 1);
                    headers.push((":status", start_event.status.as_str().as_bytes()));
                    for (k, v) in start_event.headers.iter() {
                        headers.push((k.as_str(), v.as_bytes()));
                    }
                    envoy_filter.send_response_headers(&headers, false);
                }
                let trailers_ref: Vec<(&str, &[u8])> = trailers
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_bytes()))
                    .collect();
                envoy_filter.send_response_trailers(&trailers_ref);
            }
            ResponseEvent::Exception => {
                if !self.response_closed.get() {
                    self.response_closed.set(true);
                    envoy_filter.send_response(
                        500,
                        &[
                            ("content-type", b"text/plain; charset=utf-8"),
                            ("connection", b"close"),
                        ],
                        Some(b"Internal Server Error"),
                        None,
                    );
                }
            }
        });
    }

    fn process_transport_events<EHF: EnvoyHttpFilter>(&self, envoy_filter: &mut EHF) {
        let Some(transport_bridge) = &self.transport_bridge else {
            return;
        };
        transport_bridge.process(|event| match event {
            TransportEvent::Start(event) => {
                let mut headers: Vec<(&str, &[u8])> = Vec::with_capacity(event.headers.len() + 4);
                let mut has_content_length = false;
                for (k, v) in event.headers.iter() {
                    if k == ::http::header::CONTENT_LENGTH {
                        has_content_length = true;
                    }
                    headers.push((k.as_str(), v.as_bytes()));
                }
                headers.push((":method", event.method.as_str().as_bytes()));
                headers.push((
                    ":path",
                    event.url[url::Position::BeforePath..url::Position::AfterQuery].as_bytes(),
                ));
                headers.push((":scheme", event.url.scheme().as_bytes()));
                headers.push((
                    ":authority",
                    event.url[url::Position::BeforeHost..url::Position::AfterPort].as_bytes(),
                ));
                let end_stream = event.buffered_body.is_some();
                let mut content_length_buffer = itoa::Buffer::new();
                if let Some(body) = &event.buffered_body
                    && !has_content_length
                {
                    headers.push((
                        "content-length",
                        content_length_buffer.format(body.len()).as_bytes(),
                    ));
                }
                let (res, stream_handle) = envoy_filter.start_http_stream(
                    &event.cluster_name,
                    &headers,
                    event.buffered_body.as_deref(),
                    end_stream,
                    event.timeout_ms,
                );
                if res != abi::envoy_dynamic_module_type_http_callout_init_result::Success {
                    let _ = event.response_tx.send(TransportResponse::Reset {
                        reason: ResetReason::InvalidUpstream,
                        exception: None,
                    });
                    return;
                }
                let _ = event
                    .response_tx
                    .send(TransportResponse::Started(stream_handle));
                self.transport_responses.borrow_mut().insert(
                    stream_handle,
                    TransportState {
                        response_tx: event.response_tx,
                        exception: None,
                    },
                );
            }
            TransportEvent::SendData(event) => {
                let SendStreamDataEvent {
                    stream_handle,
                    data,
                    end_stream,
                } = event;
                // Possible stream was reset before we process this event, so check for liveness.
                let live = self
                    .transport_responses
                    .borrow()
                    .contains_key(&stream_handle);
                if live {
                    unsafe {
                        envoy_filter.send_http_stream_data(stream_handle, &data, end_stream);
                    }
                }
            }
            TransportEvent::Reset(mut event) => {
                let live = {
                    let mut transport_responses = self.transport_responses.borrow_mut();
                    match transport_responses.get_mut(&event.stream_handle) {
                        Some(state) => {
                            if let Some(exception) = event.exception.take() {
                                state.exception = Some(exception);
                            }
                            true
                        }
                        None => false,
                    }
                };
                if live {
                    // Re-enters on_http_stream_reset, which removes the entry.
                    unsafe {
                        envoy_filter.reset_http_stream(event.stream_handle);
                    }
                }
            }
        });
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
