use envoy_proxy_dynamic_modules_rust_sdk::*;
use http::{HeaderName, HeaderValue};
use pyo3::Python;
use pyo3::types::PyTracebackMethods as _;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::asgi::python;
use crate::asgi::python::*;
use crate::asgi::shared::ExecutorHandles;
use crate::asgi::transport::{RequestBody, SendStreamDataEvent, TransportEvent, TransportState};
use crate::envoy::*;
use crate::eventbridge::EventBridge;
use crate::types::*;

pub struct Config {
    executor: python::Executor,
    handles: Option<ExecutorHandles>,
}

impl Config {
    pub fn new(
        app: &str,
        root_path: &str,
        constants: Arc<Constants>,
        worker_threads: usize,
        enable_lifespan: Option<bool>,
    ) -> Option<Self> {
        let (module, attr) = app.split_once(":").unwrap_or((app, "app"));
        let (executor, handles) = match python::Executor::new(
            module,
            attr,
            root_path,
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

impl<EHF: EnvoyHttpFilter> HttpFilterConfig<EHF> for Config {
    fn new_http_filter(&self, _envoy: &mut EHF) -> Box<dyn HttpFilter<EHF>> {
        Box::new(Filter {
            executor: self.executor.clone(),
            request_closed: false,
            response_closed: Arc::new(AtomicBool::new(false)),
            response_trailers: None,
            recv_bridge: EventBridge::new(),
            send_bridge: EventBridge::new(),
            transport_bridge: EventBridge::new(),
            transport_responses: HashMap::new(),
            downstream_watermark_level: 0,
        })
    }
}

struct Filter {
    executor: python::Executor,

    request_closed: bool,
    response_closed: Arc<AtomicBool>,

    response_trailers: Option<Vec<(HeaderName, HeaderValue)>>,

    recv_bridge: EventBridge<RecvFuture>,
    send_bridge: EventBridge<SendEvent>,

    transport_bridge: EventBridge<TransportEvent>,
    transport_responses: HashMap<u64, TransportState>,

    downstream_watermark_level: usize,
}

impl<EHF: EnvoyHttpFilter> HttpFilter<EHF> for Filter {
    fn on_request_headers(
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_headers_status {
        if end_of_stream {
            self.request_closed = true;
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
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_body_status {
        if end_of_stream {
            self.request_closed = true;
        }

        self.handle_read(envoy_filter);

        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::StopIterationAndBuffer
    }

    fn on_request_trailers(
        &mut self,
        envoy_filter: &mut EHF,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_trailers_status {
        self.request_closed = true;

        self.handle_read(envoy_filter);

        abi::envoy_dynamic_module_type_on_http_filter_request_trailers_status::StopIteration
    }

    fn on_stream_complete(&mut self, _envoy_filter: &mut EHF) {
        self.response_closed.store(true, Ordering::Relaxed);
        self.recv_bridge.close();
        self.send_bridge.close();
    }

    fn on_scheduled(&mut self, envoy_filter: &mut EHF, event_id: u64) {
        match event_id {
            EVENT_ID_REQUEST => {
                self.handle_read(envoy_filter);
            }
            EVENT_ID_RESPONSE => {
                if self.downstream_watermark_level == 0 {
                    self.process_send_events(envoy_filter);
                }
            }
            EVENT_ID_OUTGOING_REQUEST => {
                self.process_transport_events(envoy_filter);
            }
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
        println!(
            "Received headers for transport stream {stream_handle}, end_stream={end_stream}, has_state={}",
            self.transport_responses.contains_key(&stream_handle)
        );
        let Some(state) = self.transport_responses.get_mut(&stream_handle) else {
            println!("No state found for transport stream {stream_handle}");
            // Happens if there is a configuration bug for the upstream.
            // TODO: Surface this.
            unsafe { envoy_filter.reset_http_stream(stream_handle) };
            return;
        };
        let mut headers: Vec<(HeaderName, HeaderValue)> =
            Vec::with_capacity(response_headers.len());
        for (k, v) in response_headers {
            match (
                HeaderName::from_bytes(k.as_slice()),
                HeaderValue::from_bytes(v.as_slice()),
            ) {
                (Ok(name), Ok(value)) => headers.push((name, value)),
                // Should be validated by Envoy already so shouldn't happen in practice.
                _ => continue,
            }
        }
        let Some(future) = state.response_future.take() else {
            println!("No future found for transport stream {stream_handle}");
            // Shouldn't happen in practice.
            return;
        };
        self.executor.handle_transport_stream_started(
            stream_handle,
            headers,
            state.request_iter.take(),
            state.response_content.clone(),
            future,
            end_stream,
            self.transport_bridge.clone(),
            Box::from(envoy_filter.new_scheduler()),
        );
    }

    fn on_http_stream_data(
        &mut self,
        _envoy_filter: &mut EHF,
        stream_handle: u64,
        response_data: &[EnvoyBuffer],
        end_stream: bool,
    ) {
        let Some(state) = self.transport_responses.get_mut(&stream_handle) else {
            println!("No state found for transport stream {stream_handle}");
            // Can't happen in practice since we would have reset the stream above, but avoid panic anyways.
            return;
        };
        let len = response_data.iter().map(|b| b.as_slice().len()).sum();
        println!(
            "Received data for transport stream {stream_handle}, len={len}, end_stream={end_stream}"
        );

        let mut body = Vec::with_capacity(len);
        for buffer in response_data {
            body.extend_from_slice(buffer.as_slice());
        }
        if state.response_content.feed_response_data(body, end_stream) {
            println!("Notifying response future for transport stream {stream_handle}");
            self.executor
                .notify_response(state.response_content.clone());
        }
    }

    fn on_http_stream_trailers(
        &mut self,
        _envoy_filter: &mut EHF,
        stream_handle: u64,
        response_trailers: &[(EnvoyBuffer, EnvoyBuffer)],
    ) {
        println!(
            "Received trailers for transport stream {stream_handle}, has_state={}",
            self.transport_responses.contains_key(&stream_handle)
        );
        let Some(state) = self.transport_responses.get_mut(&stream_handle) else {
            return;
        };
        if state
            .response_content
            .feed_response_trailers(response_trailers)
        {
            self.executor
                .notify_response(state.response_content.clone());
        }
    }

    fn on_http_stream_reset(
        &mut self,
        _envoy_filter: &mut EHF,
        _stream_handle: u64,
        _reset_reason: abi::envoy_dynamic_module_type_http_stream_reset_reason,
    ) {
        println!("Transport stream reset by Envoy {_reset_reason:?}");
    }

    fn on_http_stream_complete(&mut self, _envoy_filter: &mut EHF, _stream_handle: u64) {}
}

impl Filter {
    fn handle_read(&mut self, envoy_filter: &mut impl EnvoyHttpFilter) {
        if self.request_closed || has_request_body(envoy_filter) {
            self.recv_bridge.process(|future| {
                self.executor.handle_recv_future(
                    read_request_body(envoy_filter),
                    !self.request_closed,
                    future,
                );
            });
        }
    }

    fn process_send_events(&mut self, envoy_filter: &mut impl EnvoyHttpFilter) {
        self.send_bridge.process(|event| {
            match event {
                SendEvent::Start(start_event, mut body_event) => {
                    if start_event.trailers {
                        self.response_trailers.replace(Vec::new());
                    }
                    let mut headers: Vec<(&str, &[u8])> =
                        Vec::with_capacity(start_event.headers.len() + 1);
                    headers.push((":status", start_event.status.as_str().as_bytes()));
                    for (k, v) in start_event.headers.iter() {
                        headers.push((k.as_str(), v.as_bytes()));
                    }
                    let end_stream =
                        body_event.future.is_none() && self.response_trailers.is_none();
                    if let Some(future) = body_event.future.take() {
                        self.executor.handle_send_future(future);
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
                }
                SendEvent::Body(mut event) => {
                    let end_stream = event.future.is_none() && self.response_trailers.is_none();
                    if let Some(future) = event.future.take() {
                        self.executor.handle_send_future(future);
                    }
                    envoy_filter.send_response_data(&event.body, end_stream);
                    if end_stream {}
                }
                SendEvent::Trailers(mut event) => {
                    if let Some(trailers) = &mut self.response_trailers {
                        trailers.append(&mut event.headers);
                        if !event.more_trailers {
                            let trailers_ref: Vec<(&str, &[u8])> = trailers
                                .iter()
                                .map(|(k, v)| (k.as_str(), v.as_bytes()))
                                .collect();
                            envoy_filter.send_response_trailers(trailers_ref);
                        }
                    }
                }
                SendEvent::Exception => {
                    // Will reset the stream if headers have already been sent.
                    envoy_filter.send_response(
                        500,
                        vec![
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

    fn process_transport_events(&mut self, envoy_filter: &mut impl EnvoyHttpFilter) {
        self.transport_bridge.process(|event| match event {
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
                println!(
                    "authority: {}",
                    &event.url[url::Position::BeforeHost..url::Position::AfterPort]
                );
                let (body, end_stream, request_iter) = match event.body {
                    RequestBody::Buffered(body) => (Some(body), true, None),
                    RequestBody::Iter(iter) => (None, false, Some(iter)),
                };
                let mut content_length_buffer = itoa::Buffer::new();
                if let Some(body) = &body && !has_content_length {
                    headers.push((
                        "content-length",
                        content_length_buffer.format(body.len()).as_bytes(),
                    ));
                }
                let cluster_name = if let Some(name) = event.cluster_name.as_ref() {
                    name.as_str()
                } else {
                    match event.url.scheme() {
                        "https" => "__pyvoy_default_upstream_https__",
                        _ => "__pyvoy_default_upstream_http__",
                    }
                };
                println!("Starting transport stream to cluster {cluster_name} {end_stream}");
                let (_res, stream_id) = envoy_filter.start_http_stream(
                    cluster_name,
                    headers,
                    body.as_deref(),
                    end_stream,
                    60_000,
                );
                println!(
                    "Started transport stream to cluster {cluster_name} with stream: {stream_id}, res: {_res:?}",
                );
                self.transport_responses.insert(
                    stream_id,
                    TransportState {
                        request_iter,
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
                println!(
                    "Sending data for transport stream {stream_handle}, len={}, end_stream={}",
                    data.len(),
                    end_stream
                );
                unsafe {
                    envoy_filter.send_http_stream_data(stream_handle, &data, end_stream);
                }
            }
        });
    }
}
