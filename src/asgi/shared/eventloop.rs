use std::{
    sync::{Arc, Mutex, atomic::AtomicU64, mpsc},
    thread::{self, JoinHandle},
};

use pyo3::{
    Bound, Py, PyAny, PyResult, Python,
    exceptions::PyRuntimeError,
    types::{PyAnyMethods as _, PyDict},
};

use super::lifespan::{Lifespan, execute_lifespan};
use crate::types::{Constants, SyncReceiver};

/// The async library ASGI applications run on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Io {
    /// asyncio, using uvloop, or winloop on Windows.
    Asyncio,
    /// trio, using the event loop adapter in `pyvoy._trio`.
    Trio,
}

impl Io {
    /// Parses the name of an async library, returning `None` if unsupported.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "asyncio" => Some(Self::Asyncio),
            "trio" => Some(Self::Trio),
            _ => None,
        }
    }
}

enum EventLoopsInner {
    Single(EventLoop),
    Multiple {
        loops: Box<[EventLoop]>,
        next: AtomicU64,
    },
}

/// A pool of event loops.
#[derive(Clone)]
pub(crate) struct EventLoops {
    inner: Arc<EventLoopsInner>,
}

impl EventLoops {
    /// Creates a new pool of event loops with the specified size.
    ///
    /// Each event loop has lifespan initialized using the provided ASGI parameters.
    pub(crate) fn new<'py>(
        py: Python<'py>,
        size: usize,
        app: &Bound<'py, PyAny>,
        asgi: &Bound<'py, PyDict>,
        enable_lifespan: Option<bool>,
        io: Io,
        constants: &Arc<Constants>,
    ) -> PyResult<Self> {
        match size {
            n if n > 1 => {
                let mut loops = Vec::with_capacity(n);
                for _ in 0..n {
                    loops.push(EventLoop::new(
                        py,
                        app,
                        asgi,
                        enable_lifespan,
                        io,
                        constants,
                    )?);
                }
                Ok(EventLoops {
                    inner: Arc::new(EventLoopsInner::Multiple {
                        loops: loops.into_boxed_slice(),
                        next: AtomicU64::new(0),
                    }),
                })
            }
            _ => Ok(EventLoops {
                inner: Arc::new(EventLoopsInner::Single(EventLoop::new(
                    py,
                    app,
                    asgi,
                    enable_lifespan,
                    io,
                    constants,
                )?)),
            }),
        }
    }

    /// Gets an event loop and possible lifespan state from the pool for executing
    /// the ASGI application.
    pub(crate) fn get<'py>(
        &self,
        py: Python<'py>,
    ) -> (Bound<'py, PyAny>, Option<Bound<'py, PyDict>>) {
        match self.inner.as_ref() {
            EventLoopsInner::Single(event_loop) => (
                event_loop.loop_.bind(py).clone(),
                event_loop.state.as_ref().map(|s| s.bind(py).clone()),
            ),
            // For now do simple round-robin balancing across event loops. While keeping track
            // of current requests could make sense, it's also hard to know when requests take a
            // while due to I/O, where it's not so bad to have unbalanced but fair load.
            EventLoopsInner::Multiple { loops, next } => {
                let idx = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let event_loop = &loops[(idx as usize) % loops.len()];
                (
                    event_loop.loop_.bind(py).clone(),
                    event_loop.state.as_ref().map(|s| s.bind(py).clone()),
                )
            }
        }
    }

    /// Stops the event loops, executing shutdown lifespan if needed before doing so.
    pub(crate) fn stop<'py>(&self, py: Python<'py>) -> PyResult<()> {
        match self.inner.as_ref() {
            EventLoopsInner::Single(event_loop) => event_loop.stop(py),
            EventLoopsInner::Multiple { loops, .. } => {
                for event_loop in loops.iter() {
                    event_loop.stop(py)?;
                }
                Ok(())
            }
        }
    }

    /// Joins the event loop threads.
    pub(crate) fn join(self) {
        match self.inner.as_ref() {
            EventLoopsInner::Single(event_loop) => {
                let _ = event_loop.handle.lock().unwrap().take().unwrap().join();
            }
            EventLoopsInner::Multiple { loops, .. } => {
                for event_loop in loops.iter() {
                    let _ = event_loop.handle.lock().unwrap().take().unwrap().join();
                }
            }
        }
    }
}

/// Timeout for joining event loop threads during shutdown, in seconds.
/// Recent Python includes asyncio.constants.THREAD_JOIN_TIMEOUT set
/// to this value but not older ones so we avoid reading it.
const THREAD_JOIN_TIMEOUT: usize = 300;

struct EventLoop {
    loop_: Py<PyAny>,
    handle: Mutex<Option<JoinHandle<()>>>,
    lifespan: Option<Lifespan>,
    state: Option<Py<PyDict>>,
    constants: Arc<Constants>,
}

impl EventLoop {
    fn new<'py>(
        py: Python<'py>,
        app: &Bound<'py, PyAny>,
        asgi: &Bound<'py, PyDict>,
        enable_lifespan: Option<bool>,
        io: Io,
        constants: &Arc<Constants>,
    ) -> PyResult<Self> {
        // Import the trio adapter here rather than on the event loop thread so
        // a missing trio install fails startup with its traceback.
        let new_trio_loop = match io {
            Io::Asyncio => None,
            Io::Trio => Some(
                py.import("pyvoy._trio")?
                    .getattr("new_event_loop")?
                    .unbind(),
            ),
        };
        let (tx, rx) = mpsc::channel();
        let rx = SyncReceiver::new(rx);
        let handle = thread::spawn(move || {
            let res: PyResult<()> = Python::attach(|py| {
                if let Some(new_trio_loop) = new_trio_loop {
                    // trio finalizes async generators and its worker threads
                    // itself when the run returns.
                    let loop_ = new_trio_loop.bind(py).call0()?;
                    tx.send(loop_.clone().unbind()).unwrap();
                    drop(tx);
                    loop_.call_method0("run_forever")?;
                    return Ok(());
                }

                #[cfg(not(target_family = "windows"))]
                let loop_module = py.import("uvloop")?;
                #[cfg(target_family = "windows")]
                let loop_module = py.import("winloop")?;

                let asyncio = py.import("asyncio")?;
                let loop_ = loop_module.call_method0("new_event_loop")?;
                asyncio.call_method1("set_event_loop", (&loop_,))?;
                tx.send(loop_.clone().unbind()).unwrap();
                drop(tx);
                loop_.call_method0("run_forever")?;
                let shutdown_asyncgens_coro = loop_.call_method0("shutdown_asyncgens")?;
                loop_.call_method1("run_until_complete", (shutdown_asyncgens_coro,))?;
                let shutdown_default_executor_coro =
                    loop_.call_method1("shutdown_default_executor", (THREAD_JOIN_TIMEOUT,))?;
                loop_.call_method1("run_until_complete", (shutdown_default_executor_coro,))?;
                loop_.call_method0("close")?;
                Ok(())
            });
            res.unwrap();
        });

        let loop_ = py.detach(|| rx.recv()).map_err(|e| {
            PyRuntimeError::new_err(format!(
                "Failed to initialize event loop for ASGI executor: {}",
                e
            ))
        })?;
        drop(rx);

        let (lifespan, state) = match enable_lifespan {
            Some(false) => (None, None),
            _ => execute_lifespan(
                app,
                asgi,
                loop_.bind(py),
                enable_lifespan.unwrap_or(false),
                io,
                constants,
            )?,
        };

        Ok(Self {
            loop_,
            handle: Mutex::new(Some(handle)),
            lifespan,
            state,
            constants: constants.clone(),
        })
    }

    fn stop<'py>(&self, py: Python<'py>) -> PyResult<()> {
        let loop_ = self.loop_.bind(py);

        if let Some(lifespan) = self.lifespan.as_ref() {
            lifespan.shutdown(py, loop_);
        }

        let stop = loop_.getattr(&self.constants.stop)?;
        loop_.call_method1(&self.constants.call_soon_threadsafe, (stop,))?;
        Ok(())
    }
}
