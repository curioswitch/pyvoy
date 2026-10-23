use envoy_proxy_dynamic_modules_rust_sdk::{
    CatchUnwind, EnvoyBuffer, EnvoyHttpFilter, EnvoyHttpFilterScheduler as _, HttpFilter,
    HttpFilterConfig, abi, envoy_log_error,
};
use http::{HeaderName, HeaderValue, StatusCode};
use pyo3::Python;
use pyo3::types::PyTracebackMethods as _;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::asgi::python;
use crate::asgi::python::*;
use crate::asgi::shared::ExecutorHandles;
use crate::asgi::shared::eventloop::LoopKind;
use crate::asgi::transport::{
    RequestBody, ResetReason, SendStreamDataEvent, TransportEvent, TransportState,
};
use crate::envoy::*;
use crate::eventbridge::EventBridge;
use crate::types::*;

pub struct Config {
    executor: python::Executor,
    handles: Option<ExecutorHandles>,
    enable_http_client: bool,
}

impl Config {
    pub fn new(
        app: &str,
        root_path: &str,
        constants: Arc<Constants>,
        worker_threads: usize,
        enable_lifespan: Option<bool>,
        loop_kind: LoopKind,
        enable_http_client: bool,
    ) -> Option<Self> {
        let (module, attr) = app.split_once(":").unwrap_or((app, "app"));
        let (executor, handles) = match python::Executor::new(
            module,
            attr,
            root_path,
            constants,
            worker_threads,
            enable_lifespan,
            loop_kind,
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
            enable_http_client,
        })
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        self.executor.shutdown();
        self.handles.take().unwrap().join();
    }
}

impl<EHF: EnvoyHttpFilter> HttpFilterConfig<EHF> for Config {
    fn new_http_filter(&self, _envoy: &mut EHF) -> Box<dyn HttpFilter<EHF>> {
        Box::new(CatchUnwind::new(Filter {
            executor: self.executor.clone(),
            request_closed: Cell::new(false),
            response_closed: Arc::new(AtomicBool::new(false)),
            response_trailers: RefCell::new(None),
            recv_bridge: EventBridge::new(),
            send_bridge: EventBridge::new(),
            transport_bridge: self.enable_http_client.then(EventBridge::new),
            transport_responses: RefCell::new(HashMap::new()),
            downstream_watermark_level: Cell::new(0),
        }))
    }
}

struct Filter {
    executor: python::Executor,

    request_closed: Cell<bool>,
    response_closed: Arc<AtomicBool>,

    response_trailers: RefCell<Option<Vec<(HeaderName, HeaderValue)>>>,

    recv_bridge: EventBridge<RecvFuture>,
    send_bridge: EventBridge<SendEvent>,

    /// Only present when the HTTP client is enabled.
    transport_bridge: Option<EventBridge<TransportEvent>>,
    transport_responses: RefCell<HashMap<u64, TransportState>>,

    downstream_watermark_level: Cell<usize>,
}

impl<EHF: EnvoyHttpFilter> HttpFilter<EHF> for Filter {
    fn on_request_headers(
        &self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_headers_status {
        if end_of_stream {
            self.request_closed.set(true);
        }

        let trailers_accepted = envoy_filter
            .get_request_header_value("te")
            .map(|v| v.as_slice() == b"trailers")
            .unwrap_or(false);
        let scope = new_scope(envoy_filter);

        self.executor.execute_app(
            scope,
            end_of_stream,
            trailers_accepted,
            self.response_closed.clone(),
            self.recv_bridge.clone(),
            self.send_bridge.clone(),
            self.transport_bridge.clone(),
            Box::from(envoy_filter.new_scheduler()),
        );
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

        self.handle_read(envoy_filter);

        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::StopIterationAndWatermark
    }

    fn on_request_trailers(
        &self,
        envoy_filter: &mut EHF,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_trailers_status {
        self.request_closed.set(true);

        self.handle_read(envoy_filter);

        abi::envoy_dynamic_module_type_on_http_filter_request_trailers_status::StopIteration
    }

    fn on_stream_complete(&self, _envoy_filter: &mut EHF) {
        self.response_closed.store(true, Ordering::Relaxed);
        self.recv_bridge.close();
        self.send_bridge.close();

        let transport_responses: Vec<TransportState> = self
            .transport_responses
            .borrow_mut()
            .drain()
            .map(|(_, state)| state)
            .collect();
        for state in transport_responses {
            if let Some(response_future) = state.response_future {
                self.executor.set_response_future_exception(
                    response_future,
                    ResetReason::ServerRequestDone,
                    state.response_content,
                    state.request_content,
                );
            } else if state.response_content.reset(ResetReason::ServerRequestDone) {
                self.executor.notify_response(state.response_content);
            }
        }
    }

    fn on_scheduled(&self, envoy_filter: &mut EHF, event_id: u64) {
        match event_id {
            EVENT_ID_REQUEST => {
                self.handle_read(envoy_filter);
            }
            EVENT_ID_RESPONSE => {
                if self.downstream_watermark_level.get() == 0 {
                    self.process_send_events(envoy_filter);
                }
            }
            EVENT_ID_OUTGOING_REQUEST => {
                self.process_transport_events(envoy_filter);
            }
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
        // Take what the executor needs out of the map so the borrow ends before
        // the reset below, which re-enters on_http_stream_reset.
        let state = self
            .transport_responses
            .borrow_mut()
            .get_mut(&stream_handle)
            .map(|state| {
                (
                    state.response_future.take(),
                    state.response_content.clone(),
                    state.request_content.clone(),
                )
            });
        let Some((response_future, response_content, request_content)) = state else {
            // Happens if there is a configuration bug for the upstream.
            // TODO: Surface this.
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
        let Some(future) = response_future else {
            // Shouldn't happen in practice.
            return;
        };
        self.executor.handle_transport_received_response_headers(
            status,
            headers,
            response_content,
            future,
            request_content,
            end_stream,
        );
    }

    fn on_http_stream_data(
        &self,
        _envoy_filter: &mut EHF,
        stream_handle: u64,
        response_data: &[EnvoyBuffer],
        end_stream: bool,
    ) {
        let response_content = self
            .transport_responses
            .borrow()
            .get(&stream_handle)
            .map(|state| state.response_content.clone());
        let Some(response_content) = response_content else {
            // Can't happen in practice since we would have reset the stream in on_http_stream_headers,
            // but avoid panic anyways.
            return;
        };
        let len = response_data.iter().map(|b| b.as_slice().len()).sum();
        let mut body = Vec::with_capacity(len);
        for buffer in response_data {
            body.extend_from_slice(buffer.as_slice());
        }
        if response_content.feed_response_data(body, end_stream) {
            self.executor.notify_response(response_content);
        }
    }

    fn on_http_stream_trailers(
        &self,
        _envoy_filter: &mut EHF,
        stream_handle: u64,
        response_trailers: &[(EnvoyBuffer, EnvoyBuffer)],
    ) {
        let response_content = self
            .transport_responses
            .borrow()
            .get(&stream_handle)
            .map(|state| state.response_content.clone());
        let Some(response_content) = response_content else {
            return;
        };
        if response_content.feed_response_trailers(response_trailers) {
            self.executor.notify_response(response_content);
        }
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
        if let Some(response_future) = state.response_future.take() {
            // No response headers received before resetting.
            self.executor.set_response_future_exception(
                response_future,
                reason,
                state.response_content.clone(),
                state.request_content,
            );
        }
        if state.response_content.reset(reason) {
            self.executor.notify_response(state.response_content);
        }
    }

    fn on_http_stream_complete(&self, _envoy_filter: &mut EHF, stream_handle: u64) {
        // Cleanup for a successful response. It may be more precise to do this on data with end_stream or
        // trailers but simpler to just take care of it here.
        self.transport_responses.borrow_mut().remove(&stream_handle);
    }
}

impl Filter {
    fn handle_read(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        if self.request_closed.get() || has_request_body(envoy_filter) {
            self.recv_bridge.process(|future| {
                self.executor.handle_recv_future(
                    read_request_body(envoy_filter),
                    !self.request_closed.get(),
                    future,
                );
            });
        }
    }

    fn process_send_events(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        self.send_bridge.process(|event| {
            match event {
                SendEvent::Start(start_event, mut body_event) => {
                    if start_event.trailers {
                        *self.response_trailers.borrow_mut() = Some(Vec::new());
                    }
                    let mut headers: Vec<(&str, &[u8])> =
                        Vec::with_capacity(start_event.headers.len() + 1);
                    headers.push((":status", start_event.status.as_str().as_bytes()));
                    for (k, v) in start_event.headers.iter() {
                        headers.push((k.as_str(), v.as_bytes()));
                    }
                    let end_stream =
                        body_event.future.is_none() && self.response_trailers.borrow().is_none();
                    if let Some(future) = body_event.future.take() {
                        self.executor.handle_send_future(future);
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
                SendEvent::Body(mut event) => {
                    let end_stream =
                        event.future.is_none() && self.response_trailers.borrow().is_none();
                    if let Some(future) = event.future.take() {
                        self.executor.handle_send_future(future);
                    }
                    envoy_filter.send_response_data(&event.body, end_stream);
                }
                SendEvent::Trailers(mut event) => {
                    // Accumulate outside the cell: sending the trailers ends
                    // the stream, which re-enters on_stream_complete.
                    let trailers = self.response_trailers.borrow_mut().take();
                    let Some(mut trailers) = trailers else {
                        return;
                    };
                    trailers.append(&mut event.headers);
                    if event.more_trailers {
                        *self.response_trailers.borrow_mut() = Some(trailers);
                        return;
                    }
                    let trailers_ref: Vec<(&str, &[u8])> = trailers
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_bytes()))
                        .collect();
                    envoy_filter.send_response_trailers(&trailers_ref);
                }
                SendEvent::Exception => {
                    // Will reset the stream if headers have already been sent.
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

    fn process_transport_events(&self, envoy_filter: &mut impl EnvoyHttpFilter) {
        let Some(transport_bridge) = &self.transport_bridge else {
            return;
        };
        transport_bridge.process(|event| match event {
            TransportEvent::Start(event) => {
                let mut headers: Vec<(&str, &[u8])> = Vec::with_capacity(event.headers.len() + 4);
                let mut has_content_length = false;
                for (k, v) in event.headers.iter() {
                    if k == http::header::CONTENT_LENGTH {
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
                let (body, end_stream, request_content) = match event.body {
                    RequestBody::Buffered(body) => (Some(body), true, None),
                    RequestBody::Iter(iter) => (None, false, Some(iter)),
                };
                let mut content_length_buffer = itoa::Buffer::new();
                if let Some(body) = &body
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
                    body.as_deref(),
                    end_stream,
                    event.timeout_ms,
                );
                if res != abi::envoy_dynamic_module_type_http_callout_init_result::Success {
                    self.executor.set_response_future_exception(
                        event.response_future,
                        ResetReason::InvalidUpstream,
                        event.response_content,
                        request_content,
                    );
                    return;
                }
                if let Some(request_content) = &request_content {
                    request_content.set_stream_handle(stream_handle);
                    self.executor.notify_request(request_content.clone());
                }
                event.response_content.set_stream_handle(stream_handle);
                self.transport_responses.borrow_mut().insert(
                    stream_handle,
                    TransportState {
                        request_content,
                        response_future: Some(event.response_future),
                        response_content: event.response_content,
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
                let response_content = self
                    .transport_responses
                    .borrow()
                    .get(&event.stream_handle)
                    .map(|state| state.response_content.clone());
                if let Some(response_content) = response_content {
                    if let Some(exception) = event.exception.take() {
                        response_content.set_exception(exception);
                    }
                    // Re-enters on_http_stream_reset, which removes the entry.
                    unsafe {
                        envoy_filter.reset_http_stream(event.stream_handle);
                    }
                }
            }
        });
    }
}
