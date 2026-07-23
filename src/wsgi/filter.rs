use crate::envoy::{buffers_len, extend_from_buffers, has_request_body};
use crate::eventbridge::EventBridge;
use crate::wsgi::python::Executor;
use crate::wsgi::transport::{
    ResetReason, SendStreamDataEvent, TransportEvent, TransportResponse, TransportState,
};
use ::http::{HeaderName, HeaderValue, StatusCode};
use envoy_proxy_dynamic_modules_rust_sdk::*;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, mpsc};

use super::types::*;
use crate::types::*;
pub struct Config {
    executor: Executor,
}

impl Config {
    pub fn new(
        app: &str,
        root_path: &str,
        constants: Arc<Constants>,
        worker_threads: usize,
    ) -> Option<Self> {
        let (module, attr) = app.split_once(":").unwrap_or((app, "app"));
        let executor = match Executor::new(module, attr, root_path, worker_threads, constants) {
            Ok(executor) => executor,
            Err(err) => {
                eprintln!("Failed to initialize WSGI app: {err}");
                return None;
            }
        };
        Some(Self { executor })
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
            request_closed: false,
            response_closed: false,
            request_read_bridge: EventBridge::new(),
            response_bridge: EventBridge::new(),
            transport_bridge: EventBridge::new(),
            transport_responses: HashMap::new(),
            request_body_rx: Some(request_body_rx),
            request_body_tx,
            pending_read: RequestReadEvent::Wait,
            read_buffer: Vec::new(),
            response_written_tx,
            response_written_rx: Some(response_written_rx),
            downstream_watermark_level: 0,
        }))
    }
}

struct Filter {
    executor: Executor,

    request_closed: bool,
    response_closed: bool,

    request_read_bridge: EventBridge<RequestReadEvent>,
    response_bridge: EventBridge<ResponseEvent>,
    pending_read: RequestReadEvent,
    read_buffer: Vec<u8>,

    transport_bridge: EventBridge<TransportEvent>,
    transport_responses: HashMap<u64, TransportState>,

    request_body_rx: Option<Receiver<RequestBody>>,
    request_body_tx: Sender<RequestBody>,

    response_written_tx: Sender<()>,
    response_written_rx: Option<Receiver<()>>,

    downstream_watermark_level: usize,
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
            end_of_stream,
            self.request_read_bridge.clone(),
            self.request_body_rx.take().unwrap(),
            self.response_bridge.clone(),
            self.response_written_rx.take().unwrap(),
            self.transport_bridge.clone(),
            Box::from(envoy_filter.new_scheduler()),
        );
        if end_of_stream {
            self.request_closed = true;
            abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::StopIteration
        } else {
            abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::StopIteration
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

    fn on_request_trailers(
        &mut self,
        envoy_filter: &mut EHF,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_trailers_status {
        self.request_closed = true;

        self.process_read(envoy_filter);

        abi::envoy_dynamic_module_type_on_http_filter_request_trailers_status::StopIteration
    }

    fn on_stream_complete(&mut self, _envoy_filter: &mut EHF) {
        self.response_closed = true;
        self.request_read_bridge.close();
        self.response_bridge.close();
        self.transport_bridge.close();

        for (_, state) in self.transport_responses.drain() {
            let _ = state.response_tx.send(TransportResponse::Reset {
                reason: ResetReason::ServerRequestDone,
                exception: state.exception,
            });
        }
    }

    fn on_scheduled(&mut self, envoy_filter: &mut EHF, event_id: u64) {
        match event_id {
            EVENT_ID_REQUEST => self.process_read(envoy_filter),
            EVENT_ID_RESPONSE => {
                if self.downstream_watermark_level == 0 {
                    self.process_send_events(envoy_filter);
                }
            }
            EVENT_ID_OUTGOING_REQUEST => self.process_transport_events(envoy_filter),
            _ => unreachable!(),
        }
    }

    fn on_downstream_above_write_buffer_high_watermark(&mut self, _envoy_filter: &mut EHF) {
        self.downstream_watermark_level += 1;
    }

    fn on_downstream_below_write_buffer_low_watermark(&mut self, envoy_filter: &mut EHF) {
        self.downstream_watermark_level -= 1;
        if self.downstream_watermark_level == 0 {
            envoy_filter.new_scheduler().commit(EVENT_ID_RESPONSE);
        }
    }

    fn on_http_stream_headers(
        &mut self,
        envoy_filter: &mut EHF,
        stream_handle: u64,
        response_headers: &[(EnvoyBuffer, EnvoyBuffer)],
        end_stream: bool,
    ) {
        let Some(state) = self.transport_responses.get(&stream_handle) else {
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
        let _ = state.response_tx.send(TransportResponse::Headers {
            status,
            headers,
            end_stream,
        });
    }

    fn on_http_stream_data(
        &mut self,
        _envoy_filter: &mut EHF,
        stream_handle: u64,
        response_data: &[EnvoyBuffer],
        end_stream: bool,
    ) {
        let Some(state) = self.transport_responses.get(&stream_handle) else {
            return;
        };
        let len = response_data.iter().map(|b| b.as_slice().len()).sum();
        let mut body = Vec::with_capacity(len);
        for buffer in response_data {
            body.extend_from_slice(buffer.as_slice());
        }
        if !body.is_empty() {
            let _ = state
                .response_tx
                .send(TransportResponse::Body(body.into_boxed_slice()));
        }
        if end_stream {
            let _ = state.response_tx.send(TransportResponse::End);
        }
    }

    fn on_http_stream_trailers(
        &mut self,
        _envoy_filter: &mut EHF,
        stream_handle: u64,
        response_trailers: &[(EnvoyBuffer, EnvoyBuffer)],
    ) {
        let Some(state) = self.transport_responses.get(&stream_handle) else {
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
        let _ = state
            .response_tx
            .send(TransportResponse::Trailers(trailers));
    }

    fn on_http_stream_reset(
        &mut self,
        _envoy_filter: &mut EHF,
        stream_handle: u64,
        reason: abi::envoy_dynamic_module_type_http_stream_reset_reason,
    ) {
        let Some(mut state) = self.transport_responses.remove(&stream_handle) else {
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

    fn on_http_stream_complete(&mut self, _envoy_filter: &mut EHF, stream_handle: u64) {
        // Cleanup for a successful response. It may be more precise to do this on data with
        // end_stream or trailers but simpler to just take care of it here.
        self.transport_responses.remove(&stream_handle);
    }
}

impl Filter {
    fn process_read<EHF: EnvoyHttpFilter>(&mut self, envoy_filter: &mut EHF) {
        self.request_read_bridge.process(|event| {
            self.pending_read = event;
        });
        // Reads always block until there is data or the request is finished,
        // so don't process them otherwise.
        if !has_request_body(envoy_filter) && !self.request_closed {
            return;
        }
        match self.pending_read {
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
                self.read_buffer.reserve(
                    buffers_len(&envoy_filter.get_buffered_request_body())
                        + buffers_len(&envoy_filter.get_received_request_body()),
                );
                let buffered_read = extend_from_buffers(
                    &envoy_filter.get_buffered_request_body(),
                    &mut self.read_buffer,
                );
                if buffered_read > 0 {
                    envoy_filter.drain_buffered_request_body(buffered_read);
                }
                let received_read = extend_from_buffers(
                    &envoy_filter.get_received_request_body(),
                    &mut self.read_buffer,
                );
                if received_read > 0 {
                    envoy_filter.drain_received_request_body(received_read);
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
            _ => (),
        }
    }

    fn read_request_until_line_or_size(
        &mut self,
        buffers: &Vec<EnvoyMutBuffer<'_>>,
        n: isize,
    ) -> (bool, usize) {
        let mut read = 0;
        for buffer in buffers.iter().map(|b| b.as_slice()) {
            for &b in buffer {
                self.read_buffer.push(b);
                read += 1;
                if b == b'\n' || (n > 0 && self.read_buffer.len() >= n as usize) {
                    return (true, read);
                }
            }
        }
        (false, read)
    }

    fn process_send_events<EHF: EnvoyHttpFilter>(&mut self, envoy_filter: &mut EHF) {
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
                if !self.response_closed {
                    self.response_closed = true;
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

    fn process_transport_events<EHF: EnvoyHttpFilter>(&mut self, envoy_filter: &mut EHF) {
        self.transport_bridge.process(|event| match event {
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
                self.transport_responses.insert(
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
                if self.transport_responses.contains_key(&stream_handle) {
                    unsafe {
                        envoy_filter.send_http_stream_data(stream_handle, &data, end_stream);
                    }
                }
            }
            TransportEvent::Reset(mut event) => {
                if let Some(state) = self.transport_responses.get_mut(&event.stream_handle) {
                    if let Some(exception) = event.exception.take() {
                        state.exception = Some(exception);
                    }
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
