use std::sync::{
    Arc,
    mpsc::{self, Sender},
};

use super::awaitable::{EmptyAwaitable, ErrorAwaitable, ValueAwaitable};
use crate::types::{Constants, SyncReceiver};
use envoy_proxy_dynamic_modules_rust_sdk::{envoy_log_error, envoy_log_info};
use pyo3::{
    exceptions::PyRuntimeError,
    prelude::*,
    types::{PyDict, PyString},
};

/// Result of executing the ASGI lifespan protocol.
pub(crate) struct Lifespan {
    /// The receiver of lifespan events from the app.
    pub lifespan_rx: SyncReceiver<LifespanEvent>,
    /// An asyncio.Future that will be completed with the lifespan.shutdown message.
    shutdown_future: Py<PyAny>,
    /// Memoized constants.
    constants: Arc<Constants>,
}

impl Lifespan {
    /// Initiates the shutdown process for the lifespan.
    ///
    /// Because the server is going to shutdown regardless, we handle all errors with logging here
    /// instead of returning them.
    pub(crate) fn shutdown<'py>(&self, py: Python<'py>, loop_: &Bound<'py, PyAny>) {
        if let Err(err) = self.set_shutdown_future(py, loop_) {
            // All we do above is schedule set_result, it should not fail in practice.
            envoy_log_error!(
                "Failed to start lifespan shutdown. This is usually a bug in pyvoy: {}",
                err
            );
        }

        match py.detach(|| self.lifespan_rx.recv()) {
            Ok(LifespanEvent::ShutdownComplete) => {
                envoy_log_info!("Application shutdown complete.");
            }
            Ok(LifespanEvent::ShutdownFailed(message)) => {
                envoy_log_error!("Application shutdown failed: {}", message);
            }
            // Follow uvicorn's semantics of ignoring if a coroutine doesn't send events.
            Ok(LifespanEvent::LifespanComplete) => {}
            Ok(LifespanEvent::LifespanFailed(message)) => {
                envoy_log_error!("Lifespan exception during shutdown: {}", message);
            }
            // The send callable validates event order so we shouldn't get anything else here.
            Ok(_) => unreachable!(),
            Err(_) => {
                // Shouldn't happen in practice
                envoy_log_error!("Unexpected error during lifespan shutdown.");
            }
        }
    }

    fn set_shutdown_future<'py>(&self, py: Python<'py>, loop_: &Bound<'py, PyAny>) -> PyResult<()> {
        let set_result = self
            .shutdown_future
            .getattr(py, &self.constants.set_result)?;
        let event = PyDict::new(py);
        event.set_item(&self.constants.typ, &self.constants.lifespan_shutdown)?;
        loop_.call_method1(&self.constants.call_soon_threadsafe, (set_result, event))?;
        Ok(())
    }
}

/// A lifespan event sent from the application to the server.
pub(crate) enum LifespanEvent {
    /// The 'lifetime.startup.complete' event.
    StartupComplete,
    /// The 'lifespan.startup.failed' event with an optional error message.
    StartupFailed(String),
    /// The 'lifespan.shutdown.complete' event.
    ShutdownComplete,
    /// The 'lifespan.shutdown.failed' event with an optional error message.
    ShutdownFailed(String),
    /// The lifespan call failed completely, meaning the app doesn't support it.
    LifespanFailed(String),
    /// The lifespan coroutine completed without an exception. This is only valid
    /// after sending either ShutdownComplete or ShutdownFailed, and any other time
    /// indicates a bad application.
    LifespanComplete,
}

/// Executes the ASGI app with a lifespan scope.
///
/// This function runs the coroutine and sends a startup event.
///
/// - If it raises an exception, lifespan is considered unsupported and we return no lifespan.
/// - If it completes successfully, we return a Lifespan object to manage shutdown later.
/// - If the startup event receives a failure message, we return an error to prevent Envoy startup.
pub(crate) fn execute_lifespan<'py>(
    app: &Bound<'py, PyAny>,
    asgi: &Bound<'py, PyDict>,
    loop_: &Bound<'py, PyAny>,
    require_lifespan: bool,
    constants: &Arc<Constants>,
) -> PyResult<(Option<Lifespan>, Option<Py<PyDict>>)> {
    let py = app.py();

    let scope = PyDict::new(py);
    scope.set_item(&constants.typ, &constants.lifespan)?;
    scope.set_item(&constants.asgi, asgi)?;

    let state = PyDict::new(py);
    scope.set_item(&constants.state, &state)?;

    let (lifespan_tx, lifespan_rx) = mpsc::channel::<LifespanEvent>();
    let lifespan_rx = SyncReceiver::new(lifespan_rx);
    let shutdown_future = loop_.call_method0(&constants.create_future)?;

    let recv_callable = RecvCallable {
        next_event: NextLifespanEvent::Startup,
        shutdown_future: shutdown_future.clone().unbind(),
        constants: constants.clone(),
    };

    let send_callable = SendCallable {
        next_event: NextLifespanEvent::Startup,
        lifespan_tx: lifespan_tx.clone(),
        constants: constants.clone(),
    };

    let coro = match app.call1((scope, recv_callable, send_callable)) {
        Ok(coro) => coro,
        Err(e) => {
            // While coroutines should generally not raise exceptions immediately, if they do
            // we treat it the same as LifespanFailed.
            if require_lifespan {
                let tb = e.traceback(py).unwrap().format().unwrap_or_default();
                let msg = format!("Exception in ASGI lifespan\n{}{}", tb, e);
                envoy_log_error!("{}", msg);
                return Err(PyRuntimeError::new_err(
                    "Application startup failed. Exiting.",
                ));
            } else {
                envoy_log_info!("ASGI 'lifespan' protocol appears unsupported.");
                return Ok((None, None));
            }
        }
    };

    let asyncio = py.import(&constants.asyncio)?;
    let future = asyncio.call_method1(&constants.run_coroutine_threadsafe, (coro, loop_))?;
    future.call_method1(
        &constants.add_done_callback,
        (FutureHandler {
            lifespan_tx,
            constants: constants.clone(),
        },),
    )?;

    let lifespan_supported = py.detach(|| {
        match lifespan_rx.recv().unwrap() {
            LifespanEvent::StartupComplete => {
                envoy_log_info!("Application startup complete.");
                Ok(true)
            }
            LifespanEvent::StartupFailed(err) => Err(PyRuntimeError::new_err(format!(
                "Application startup failed: {}",
                err
            ))),
            LifespanEvent::LifespanFailed(msg) => {
                if require_lifespan {
                    envoy_log_error!("{}", msg);
                    Err(PyRuntimeError::new_err(
                        "Application startup failed. Exiting.",
                    ))
                } else {
                    envoy_log_info!("ASGI 'lifespan' protocol appears unsupported.");
                    Ok(false)
                }
            }
            // The ASGI spec does not document how to handle a coroutine that completes without exception
            // but uvicorn seems to treat it as explicitly ignoring lifespan and does not log or fail.
            LifespanEvent::LifespanComplete => Ok(false),
            // The send callabale validates event types so we know we won't have other events here.
            _ => unreachable!(),
        }
    })?;

    if lifespan_supported {
        Ok((
            Some(Lifespan {
                shutdown_future: shutdown_future.unbind(),
                constants: constants.clone(),
                lifespan_rx: SyncReceiver::new(lifespan_rx.take()),
            }),
            Some(state.unbind()),
        ))
    } else {
        Ok((None, None))
    }
}

/// The state machine for the lifespan.
///
/// In general, it should only be called twice, once to receive the startup event
/// and the other shutdown. There is no specification for how to deal with unusual
/// scenarios such as calling recv multiple times without a send. As lifespan is
/// an advanced protocol, we go ahead and assume they follow the standard pattern
/// with undefined behavior otherwise such as hanging applications. This seems to
/// follow the pattern of other ASGI servers.
enum NextLifespanEvent {
    /// Waiting to receive the 'lifespan.startup' event.
    Startup,
    /// Waiting to receive the 'lifespan.shutdown' event.
    Shutdown,
    /// All events have been processed.
    Completed,
}

/// Callable for the receive function in the lifespan protocol.
#[pyclass(module = "_pyvoy.asgi.lifespan")]
struct RecvCallable {
    /// The next lifespan event to expect.
    next_event: NextLifespanEvent,
    /// The future to complete with 'lifespan.shutdown' when the server is closing.
    shutdown_future: Py<PyAny>,
    /// Memoized constants.
    constants: Arc<Constants>,
}

unsafe impl Sync for RecvCallable {}

#[pymethods]
impl RecvCallable {
    fn __call__<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match &self.next_event {
            // https://asgi.readthedocs.io/en/latest/specs/lifespan.html#startup-receive-event
            NextLifespanEvent::Startup => {
                self.next_event = NextLifespanEvent::Shutdown;
                let event = PyDict::new(py);
                event.set_item(&self.constants.typ, &self.constants.lifespan_startup)?;
                ValueAwaitable::new_py(py, event.into_any().unbind())
            }
            // https://asgi.readthedocs.io/en/latest/specs/lifespan.html#shutdown-receive-event
            NextLifespanEvent::Shutdown => {
                self.next_event = NextLifespanEvent::Completed;
                Ok(self.shutdown_future.bind(py).clone())
            }
            NextLifespanEvent::Completed => Err(PyRuntimeError::new_err(
                "lifespan receive called after shutdown completed.",
            )),
        }
    }
}

/// Callable for the send function in the lifespan protocol.
#[pyclass(module = "_pyvoy.asgi.lifespan")]
struct SendCallable {
    /// The current lifespan state.
    next_event: NextLifespanEvent,
    /// Bridge to send lifespan events back to the server.
    lifespan_tx: Sender<LifespanEvent>,
    /// Memoized constants.
    constants: Arc<Constants>,
}

unsafe impl Sync for SendCallable {}

#[pymethods]
impl SendCallable {
    fn __call__<'py>(
        &mut self,
        py: Python<'py>,
        event: Bound<'py, PyDict>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let event_type_py = event
            .get_item(&self.constants.typ)?
            .ok_or_else(|| PyRuntimeError::new_err("Unexpected ASGI message, missing 'type'."))?;
        let event_type = event_type_py.cast::<PyString>()?.to_str()?;

        match &self.next_event {
            NextLifespanEvent::Startup => {
                match event_type {
                    // https://asgi.readthedocs.io/en/latest/specs/lifespan.html#startup-complete-send-event
                    "lifespan.startup.complete" => {
                        let _ = self.lifespan_tx.send(LifespanEvent::StartupComplete);
                    }
                    // https://asgi.readthedocs.io/en/latest/specs/lifespan.html#startup-failed-send-event
                    "lifespan.startup.failed" => {
                        let message_py = event.get_item(&self.constants.message)?;
                        let message = if let Some(msg) = message_py {
                            msg.cast::<PyString>()?.to_str()?.to_string()
                        } else {
                            String::new()
                        };
                        let _ = self.lifespan_tx.send(LifespanEvent::StartupFailed(message));
                    }
                    _ => {
                        return ErrorAwaitable::new_py(
                            py,
                            PyRuntimeError::new_err(format!(
                                "Expected lifespan message 'lifespan.startup.complete' or 'lifespan.startup.failed', got '{}'",
                                event_type
                            )),
                        );
                    }
                }
                self.next_event = NextLifespanEvent::Shutdown;
            }
            NextLifespanEvent::Shutdown => {
                match event_type {
                    // https://asgi.readthedocs.io/en/latest/specs/lifespan.html#shutdown-complete-send-event
                    "lifespan.shutdown.complete" => {
                        let _ = self.lifespan_tx.send(LifespanEvent::ShutdownComplete);
                    }
                    // https://asgi.readthedocs.io/en/latest/specs/lifespan.html#shutdown-failed-send-event
                    "lifespan.shutdown.failed" => {
                        let message_py = event.get_item(&self.constants.message)?;
                        let message = if let Some(msg) = message_py {
                            msg.cast::<PyString>()?.to_str()?.to_string()
                        } else {
                            String::new()
                        };
                        let _ = self
                            .lifespan_tx
                            .send(LifespanEvent::ShutdownFailed(message));
                    }
                    _ => {
                        return ErrorAwaitable::new_py(
                            py,
                            PyRuntimeError::new_err(format!(
                                "Expected lifespan message 'lifespan.shutdown.complete' or 'lifespan.shutdown.failed', got '{}'",
                                event_type
                            )),
                        );
                    }
                }
                self.next_event = NextLifespanEvent::Completed;
            }
            NextLifespanEvent::Completed => {
                return ErrorAwaitable::new_py(
                    py,
                    PyRuntimeError::new_err("lifespan send called after shutdown completed."),
                );
            }
        }
        EmptyAwaitable::new_py(py)
    }
}

/// Handler for the lifespan coroutine future.
///
/// Usually the coroutine won't complete until the server is shutting down,
/// but if it doesn't support lifespan at all it will raise an exception immediately
/// which indicates the server should continue without lifespan.
#[pyclass(module = "_pyvoy.asgi.lifespan")]
struct FutureHandler {
    lifespan_tx: Sender<LifespanEvent>,
    constants: Arc<Constants>,
}

unsafe impl Sync for FutureHandler {}

#[pymethods]
impl FutureHandler {
    fn __call__<'py>(&self, py: Python<'py>, future: Bound<'py, PyAny>) {
        match future.call_method1(&self.constants.result, (0,)) {
            Ok(_) => {
                let _ = self.lifespan_tx.send(LifespanEvent::LifespanComplete);
            }
            Err(e) => {
                let tb = e.traceback(py).unwrap().format().unwrap_or_default();
                let msg = format!("Exception in ASGI lifespan\n{}{}", tb, e);
                let _ = self.lifespan_tx.send(LifespanEvent::LifespanFailed(msg));
            }
        }
    }
}
