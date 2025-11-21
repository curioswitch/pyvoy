use pyo3::{exceptions::PyRuntimeError, prelude::*, types::PyTuple};

use super::types::*;
use crate::types::*;
use crate::{envoy::SyncScheduler, eventbridge::EventBridge};
use std::sync::{Arc, Mutex};

struct ResponseSenderInner {
    /// The event bridge to send response events.
    response_bridge: EventBridge<ResponseEvent>,
    /// The scheduler to wake up the filter to process response events.
    scheduler: Arc<SyncScheduler>,

    /// The start event created in start_response if not sent yet.
    start_event: Option<ResponseStartEvent>,

    /// Whether headers have been sent yet.
    headers_sent: bool,

    /// Memoized constants.
    constants: Arc<Constants>,
}

/// A wrapper around the response event bridge to allow tracking events. This helps
/// implement certain quirky features such as exc_info and write().
#[derive(Clone)]
pub(crate) struct ResponseSender {
    inner: Arc<Mutex<ResponseSenderInner>>,
}

/// Similar to ResponseEvent but we keep start and body separate to allow multiple starts.
pub(crate) enum ResponseSenderEvent {
    /// The start of the response via start_response.
    Start(ResponseStartEvent),
    /// A body chunk. If the first one, this will cause the start to be sent.
    Body(ResponseBodyEvent),
    /// An exception.
    Exception,
}

impl ResponseSender {
    pub(crate) fn new(
        response_bridge: EventBridge<ResponseEvent>,
        scheduler: Arc<SyncScheduler>,
        constants: Arc<Constants>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ResponseSenderInner {
                response_bridge,
                scheduler,
                start_event: None,
                headers_sent: false,
                constants,
            })),
        }
    }

    pub(crate) fn send<'py>(
        &mut self,
        event: ResponseSenderEvent,
        exc_info: Option<Bound<'py, PyTuple>>,
    ) -> PyResult<()> {
        let mut inner = self.inner.lock().unwrap();
        match event {
            ResponseSenderEvent::Start(start) => {
                if let Some(exc_info) = exc_info {
                    if inner.headers_sent {
                        let e = exc_info.get_item(1)?;
                        e.call_method1(&inner.constants.with_traceback, (exc_info.get_item(2)?,))?;
                        return Err(PyErr::from_value(e));
                    }
                } else if inner.start_event.is_some() {
                    return Err(PyRuntimeError::new_err(
                        "start_response called twice without exc_info",
                    ));
                }
                inner.start_event = Some(start);
                return Ok(());
            }
            ResponseSenderEvent::Body(body) => {
                if let Some(start) = inner.start_event.take() {
                    // First body event, need to send start first.
                    inner.headers_sent = true;
                    if inner
                        .response_bridge
                        .send(ResponseEvent::Start(start, body))
                        .is_ok()
                    {
                        inner.scheduler.commit(EVENT_ID_RESPONSE);
                    }
                } else {
                    // Normal body event.
                    if !inner.headers_sent {
                        return Err(PyRuntimeError::new_err(
                            "start_response not called from WSGI application",
                        ));
                    }
                    if inner
                        .response_bridge
                        .send(ResponseEvent::Body(body))
                        .is_ok()
                    {
                        inner.scheduler.commit(EVENT_ID_RESPONSE);
                    }
                }
            }
            ResponseSenderEvent::Exception => {
                if inner.response_bridge.send(ResponseEvent::Exception).is_ok() {
                    inner.scheduler.commit(EVENT_ID_EXCEPTION);
                }
            }
        }

        Ok(())
    }
}
