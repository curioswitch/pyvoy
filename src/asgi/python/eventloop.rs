use std::{
    sync::{Arc, Mutex, atomic::AtomicU64, mpsc},
    thread::{self, JoinHandle},
};

use pyo3::{
    Bound, Py, PyAny, PyResult, Python,
    exceptions::PyRuntimeError,
    types::{PyAnyMethods as _, PyDict},
};

use crate::{
    asgi::python::lifespan::{Lifespan, execute_lifespan},
    types::{Constants, SyncReceiver},
};

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
        constants: &Arc<Constants>,
    ) -> PyResult<Self> {
        match size {
            n if n > 1 => {
                let mut loops = Vec::with_capacity(n);
                for _ in 0..n {
                    loops.push(EventLoop::new(py, app, asgi, constants)?);
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
                    py, app, asgi, constants,
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
        constants: &Arc<Constants>,
    ) -> PyResult<Self> {
        let (tx, rx) = mpsc::channel();
        let rx = SyncReceiver::new(rx);
        let handle = thread::spawn(|| {
            let res: PyResult<()> = Python::attach(|py| {
                let uvloop = py.import("uvloop")?;
                let asyncio = py.import("asyncio")?;
                let loop_ = uvloop.call_method0("new_event_loop")?;
                asyncio.call_method1("set_event_loop", (&loop_,))?;
                tx.send(loop_.clone().unbind()).unwrap();
                drop(tx);
                loop_.call_method0("run_forever")?;
                Ok(())
            });
            res.unwrap();
        });

        let loop_ = py.detach(|| rx.recv()).map_err(|e| {
            PyRuntimeError::new_err(format!(
                "Failed to initialize asyncio event loop for ASGI executor: {}",
                e
            ))
        })?;
        drop(rx);

        let (lifespan, state) = execute_lifespan(app, asgi, loop_.bind(py), constants)?;

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
