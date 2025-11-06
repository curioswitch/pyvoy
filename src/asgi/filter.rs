use envoy_proxy_dynamic_modules_rust_sdk::*;
use http::{HeaderName, HeaderValue};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, mpsc};

use crate::asgi::python;
use crate::asgi::python::*;
use crate::envoy::*;
use crate::types::*;

pub struct Config {
    executor: python::Executor,
}

impl Config {
    pub fn new(app: &str, constants: Arc<Constants>) -> Option<Self> {
        let (module, attr) = app.split_once(":").unwrap_or((app, "app"));
        let executor = match python::Executor::new(module, attr, constants) {
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
        let (recv_tx, recv_rx) = mpsc::channel::<RecvFuture>();
        let (send_tx, send_rx) = mpsc::channel::<SendEvent>();
        Box::new(Filter {
            executor: self.executor.clone(),
            request_closed: false,
            response_closed: false,
            response_trailers: None,
            recv_rx,
            recv_tx: Some(recv_tx),
            send_rx,
            send_tx: Some(send_tx),
        })
    }
}

struct Filter {
    executor: python::Executor,

    request_closed: bool,
    response_closed: bool,

    response_trailers: Option<Vec<(HeaderName, HeaderValue)>>,

    recv_rx: Receiver<RecvFuture>,
    recv_tx: Option<Sender<RecvFuture>>,
    send_rx: Receiver<SendEvent>,
    send_tx: Option<Sender<SendEvent>>,
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
            self.recv_tx.take().unwrap(),
            self.send_tx.take().unwrap(),
            SyncScheduler::new(envoy_filter.new_scheduler()),
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

        if let Ok(future) = self.recv_rx.try_recv() {
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
                && let Ok(future) = self.recv_rx.try_recv()
            {
                self.executor.handle_recv_future(
                    read_request_body(envoy_filter),
                    !self.request_closed,
                    future,
                );
            }
            return;
        }
        for event in self.send_rx.try_iter() {
            match event {
                SendEvent::Start(start_event, mut body_event) => {
                    if start_event.trailers {
                        self.response_trailers.replace(Vec::new());
                    }
                    let mut status_buf = itoa::Buffer::new();
                    let mut headers: Vec<(&str, &[u8])> =
                        Vec::with_capacity(start_event.headers.len() + 1);
                    headers.push((":status", status_buf.format(start_event.status).as_bytes()));
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
                        return;
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
                    if end_stream {
                        return;
                    }
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
                            return;
                        }
                    }
                }
                SendEvent::Exception => {
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
                        return;
                    }
                }
            }
        }
    }
}
