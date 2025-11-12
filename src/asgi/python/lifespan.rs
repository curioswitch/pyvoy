use std::sync::{
    Arc,
    mpsc::{self, Receiver, Sender},
};

use crate::{
    asgi::python::awaitable::{EmptyAwaitable, ErrorAwaitable, ValueAwaitable},
    types::Constants,
};
use envoy_proxy_dynamic_modules_rust_sdk::{envoy_log_error, envoy_log_info};
use pyo3::{
    exceptions::PyRuntimeError,
    prelude::*,
    types::{PyDict, PyString},
};

/// Result of executing the ASGI lifespan protocol.
pub(crate) struct Lifespan {
    /// The state dict for the lifespan scope.
    pub state: Option<Py<PyDict>>,
    /// The receiver of lifespan events from the app.
    pub lifespan_rx: Receiver<LifespanEvent>,
    /// An asyncio.Future that will be completed with the lifespan.shutdown message.
    shutdown_future: Py<PyAny>,
    /// The event loop.
    loop_: Py<PyAny>,
    /// Memoized constants.
    constants: Arc<Constants>,
}

impl Lifespan {
    /// Initializes the startup process for the lifespan.
    pub(crate) fn startup(&self) -> PyResult<bool> {
        match self.lifespan_rx.recv().unwrap() {
            LifespanEvent::StartupComplete => {
                envoy_log_info!("Application startup complete.");
                Ok(true)
            }
            LifespanEvent::StartupFailed(err) => Err(PyRuntimeError::new_err(format!(
                "Application startup failed: {}",
                err
            ))),
            // TODO: Add option to force lifespan and log this error if it's enabled.
            LifespanEvent::LifespanFailed(_) => Ok(false),
            // The send callabale validates event types so we know we won't have other events here.
            _ => unreachable!(),
        }
    }

    /// Initiates the shutdown process for the lifespan.
    ///
    /// Because the server is going to shutdown regardless, we handle all errors with logging here
    /// instead of returning them.
    pub(crate) fn shutdown(&self) {
        if let Err(err) = Python::attach::<_, PyResult<()>>(|py| {
            let set_result = self
                .shutdown_future
                .getattr(py, &self.constants.set_result)?;
            let event = PyDict::new(py);
            event.set_item(&self.constants.typ, &self.constants.lifespan_shutdown)?;
            self.loop_.call_method1(
                py,
                &self.constants.call_soon_threadsafe,
                (set_result, event),
            )?;
            Ok(())
        }) {
            // All we do above is schedule set_result, it should not fail in practice.
            envoy_log_error!(
                "Failed to start lifespan shutdown. This is usually a bug in pyvoy: {}",
                err
            );
        }

        match self.lifespan_rx.recv() {
            Ok(LifespanEvent::ShutdownComplete) => {
                envoy_log_info!("Application shutdown complete.");
            }
            Ok(LifespanEvent::ShutdownFailed(message)) => {
                envoy_log_error!("Application shutdown failed: {}", message);
            }
            // The send callable validates event order so we shouldn't get anything else here.
            Ok(_) => unreachable!(),
            Err(_) => {
                envoy_log_error!("Lifespan coroutine terminated without sending shutdown event",);
            }
        }
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
}

/// Executes the ASGI lifespan protocol.
///
/// This function:
/// 1. Creates a lifespan scope
/// 2. Calls the app with the scope and send/receive callables
/// 3. Sends a lifespan.startup message
/// 4. Waits for the app to complete startup
/// 5. Returns the state dict and a future for shutdown
///
/// If the app raises an exception during execution, an error is sent back.
pub(crate) fn execute_lifespan<'py>(
    app: &Bound<'py, PyAny>,
    asgi: &Bound<'py, PyDict>,
    loop_: &Bound<'py, PyAny>,
    constants: &Arc<Constants>,
) -> PyResult<Lifespan> {
    let py = app.py();

    let scope = PyDict::new(py);
    scope.set_item(&constants.typ, &constants.lifespan)?;
    scope.set_item(&constants.asgi, asgi)?;

    let state = PyDict::new(py);
    scope.set_item(&constants.state, &state)?;

    let (lifespan_tx, lifespan_rx) = mpsc::channel::<LifespanEvent>();
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
            let tb = e.traceback(py).unwrap().format().unwrap_or_default();
            // While coroutines should generally not raise exceptions immediately, if they do
            // we treat it as a failure to support lifespan.
            let msg = format!("Exception in ASGI lifespan\n{}{}", tb, e);
            let _ = lifespan_tx.send(LifespanEvent::LifespanFailed(msg));
            return Ok(Lifespan {
                state: Some(state.unbind()),
                shutdown_future: shutdown_future.unbind(),
                loop_: loop_.clone().unbind(),
                constants: constants.clone(),
                lifespan_rx,
            });
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

    Ok(Lifespan {
        state: Some(state.unbind()),
        shutdown_future: shutdown_future.unbind(),
        loop_: loop_.clone().unbind(),
        constants: constants.clone(),
        lifespan_rx,
    })
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
#[pyclass]
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
#[pyclass]
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
                self.next_event = NextLifespanEvent::Shutdown;
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
            }
            NextLifespanEvent::Shutdown => {
                self.next_event = NextLifespanEvent::Completed;
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
#[pyclass]
struct FutureHandler {
    lifespan_tx: Sender<LifespanEvent>,
    constants: Arc<Constants>,
}

unsafe impl Sync for FutureHandler {}

#[pymethods]
impl FutureHandler {
    fn __call__<'py>(&self, py: Python<'py>, future: Bound<'py, PyAny>) {
        if let Err(e) = future.call_method1(&self.constants.result, (0,)) {
            let tb = e.traceback(py).unwrap().format().unwrap_or_default();
            let msg = format!("Exception in ASGI lifespan\n{}{}", tb, e);
            let _ = self.lifespan_tx.send(LifespanEvent::LifespanFailed(msg));
        }
    }
}
