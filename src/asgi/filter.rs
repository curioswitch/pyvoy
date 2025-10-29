use envoy_proxy_dynamic_modules_rust_sdk::*;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};

use crate::asgi::python;
use crate::asgi::python::*;
use crate::types::*;

pub struct Config {
    executor: python::Executor,
}

impl Config {
    pub fn new(app: &str) -> Option<Self> {
        let (module, attr) = app.split_once(":").unwrap_or((app, "app"));
        let executor = match python::Executor::new(module, attr) {
            Ok(executor) => executor,
            Err(err) => {
                eprintln!("Failed to initialize ASGI app: {err}");
                return None;
            }
        };
        Some(Self { executor })
    }
}

impl<EHF: EnvoyHttpFilter> HttpFilterConfig<EHF> for Config {
    fn new_http_filter(&mut self, _envoy: &mut EHF) -> Box<dyn HttpFilter<EHF>> {
        let (recv_future_tx, recv_future_rx) = mpsc::channel::<RecvFuture>();
        let (response_tx, response_rx) = mpsc::channel::<ResponseEvent>();
        Box::new(Filter {
            executor: self.executor.clone(),
            request_closed: false,
            response_state: ResponseState::Started,
            response_trailers: None,
            recv_future_rx,
            recv_future_tx: Some(recv_future_tx),
            response_rx,
            response_tx: Some(response_tx),
        })
    }
}

enum ResponseState {
    Started,
    SentHeaders,
    Complete,
}

struct Filter {
    executor: python::Executor,

    request_closed: bool,
    response_state: ResponseState,

    response_trailers: Option<Vec<(Box<str>, Box<[u8]>)>>,

    recv_future_rx: Receiver<RecvFuture>,
    recv_future_tx: Option<Sender<RecvFuture>>,
    response_rx: Receiver<ResponseEvent>,
    response_tx: Option<Sender<ResponseEvent>>,
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

        let mut trailers_accepted = false;
        for (name, value) in envoy_filter.get_request_headers() {
            if name.as_slice() == b"te" && value.as_slice() == b"trailers" {
                // Allow te header to upstream
                trailers_accepted = true;
                break;
            }
        }
        let scope = new_scope(envoy_filter);
        self.executor.execute_app(
            scope,
            end_of_stream,
            trailers_accepted,
            self.recv_future_tx.take().unwrap(),
            self.response_tx.take().unwrap(),
            envoy_filter.new_scheduler(),
            envoy_filter.new_scheduler(),
            envoy_filter.new_scheduler(),
        );
        abi::envoy_dynamic_module_type_on_http_filter_request_headers_status::Continue
    }

    fn on_request_body(
        &mut self,
        envoy_filter: &mut EHF,
        end_of_stream: bool,
    ) -> abi::envoy_dynamic_module_type_on_http_filter_request_body_status {
        if end_of_stream {
            self.request_closed = true;
        }

        if let Ok(future) = self.recv_future_rx.try_recv() {
            self.executor.handle_recv_future(
                read_request_body(envoy_filter),
                !end_of_stream,
                future,
            );
        }
        abi::envoy_dynamic_module_type_on_http_filter_request_body_status::StopIterationAndBuffer
    }

    fn on_scheduled(&mut self, envoy_filter: &mut EHF, event_id: u64) {
        if event_id == EVENT_ID_REQUEST {
            if (self.request_closed || has_request_body(envoy_filter))
                && let Ok(future) = self.recv_future_rx.try_recv()
            {
                self.executor.handle_recv_future(
                    read_request_body(envoy_filter),
                    !self.request_closed,
                    future,
                );
            }
            return;
        }
        if let Ok(event) = self.response_rx.try_recv() {
            match event {
                ResponseEvent::Start(start_event, mut body_event) => {
                    if start_event.trailers {
                        self.response_trailers.replace(Vec::new());
                    }
                    let headers_ref: Vec<(&str, &[u8])> = start_event
                        .headers
                        .iter()
                        .map(|(k, v)| (&k[..], &v[..]))
                        .collect();
                    let end_stream =
                        body_event.future.is_none() && self.response_trailers.is_none();
                    if let Some(future) = body_event.future.take() {
                        self.executor.handle_send_future(future);
                    }
                    if end_stream {
                        if body_event.body.is_empty() {
                            envoy_filter.send_response_headers(headers_ref, true);
                        } else {
                            envoy_filter.send_response_headers(headers_ref, false);
                            envoy_filter.send_response_data(&body_event.body, true);
                        }
                    } else {
                        envoy_filter.send_response_headers(headers_ref, false);
                        envoy_filter.send_response_data(&body_event.body, false);
                    }
                    self.response_state = ResponseState::SentHeaders;
                }
                ResponseEvent::Body(mut event) => {
                    let end_stream = event.future.is_none() && self.response_trailers.is_none();
                    if let Some(future) = event.future.take() {
                        self.executor.handle_send_future(future);
                    }
                    envoy_filter.send_response_data(&event.body, end_stream);
                }
                ResponseEvent::Trailers(mut event) => {
                    if let Some(trailers) = &mut self.response_trailers {
                        trailers.append(&mut event.headers);
                        if !event.more_trailers {
                            let trailers_ref: Vec<(&str, &[u8])> =
                                trailers.iter().map(|(k, v)| (&k[..], &v[..])).collect();
                            envoy_filter.send_response_trailers(trailers_ref);
                        }
                    }
                }
                ResponseEvent::Exception => match self.response_state {
                    // While technically it's possible we have a chance to interrupt the response to
                    // trigger a client failure, not always. It is more consistent to always allow it
                    // to be completed successfully on the client side rather than having inconsistent
                    // behavior. The exception itself was logged from Python.
                    ResponseState::Complete => {}
                    _ => {
                        self.response_state = ResponseState::Complete;
                        envoy_filter.send_response(
                            500,
                            vec![
                                ("content-type", b"text/plain; charset=utf-8"),
                                ("connection", b"close"),
                            ],
                            Some(b"Internal Server Error"),
                        );
                    }
                },
            }
        }
    }
}

fn read_request_body<EHF: EnvoyHttpFilter>(envoy_filter: &mut EHF) -> Box<[u8]> {
    if let Some(buffers) = envoy_filter.get_request_body() {
        match buffers.len() {
            0 => Box::new([]),
            1 => {
                let body = buffers[0].as_slice().to_vec().into_boxed_slice();
                envoy_filter.drain_request_body(body.len());
                body
            }
            _ => {
                let mut body_len = 0;
                for buffer in &buffers {
                    body_len += buffer.as_slice().len();
                }
                let mut body = Vec::with_capacity(body_len);
                for buffer in buffers {
                    body.extend_from_slice(buffer.as_slice());
                }
                envoy_filter.drain_request_body(body.len());
                body.into_boxed_slice()
            }
        }
    } else {
        Box::new([])
    }
}
