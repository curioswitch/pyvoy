use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock, atomic::AtomicU64, mpsc},
    thread::{self, JoinHandle},
};

use pyo3::{
    Bound, Py, PyAny, PyErr, PyRef, PyResult, Python,
    exceptions::PyRuntimeError,
    pyclass, pymethods,
    sync::PyOnceLock,
    types::{PyAnyMethods as _, PyDict, PyDictMethods as _},
};

use super::lifespan::{Lifespan, execute_lifespan};
use crate::types::{Constants, SyncReceiver};

/// The event loop ASGI applications run on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoopKind {
    /// The standard library asyncio event loop.
    Asyncio,
    /// The uvloop asyncio event loop.
    Uvloop,
    /// The winloop asyncio event loop.
    Winloop,
    /// The zuvloop asyncio event loop.
    Zuvloop,
    /// trio, using the event loop adapter in `pyvoy._trio`.
    Trio,
}

impl LoopKind {
    /// Parses the name of an event loop, returning `None` if unsupported.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "asyncio" => Some(Self::Asyncio),
            "uvloop" => Some(Self::Uvloop),
            "winloop" => Some(Self::Winloop),
            "zuvloop" => Some(Self::Zuvloop),
            "trio" => Some(Self::Trio),
            _ => None,
        }
    }

    /// The event loop used when none is configured: uvloop, or winloop on
    /// Windows.
    pub(crate) fn platform_default() -> Self {
        if cfg!(target_family = "windows") {
            Self::Winloop
        } else {
            Self::Uvloop
        }
    }

    /// The module providing `new_event_loop` for this event loop.
    fn module(self) -> &'static str {
        match self {
            Self::Asyncio => "asyncio",
            Self::Uvloop => "uvloop",
            Self::Winloop => "winloop",
            Self::Zuvloop => "zuvloop",
            Self::Trio => "pyvoy._trio",
        }
    }

    /// Returns whether the event loop can watch a socket with `add_reader`,
    /// which lets Envoy threads wake it without the GIL.
    fn watches_sockets(self) -> bool {
        match self {
            // trio has no add_reader.
            Self::Trio => false,
            // The default asyncio event loop on Windows does not support add_reader.
            Self::Asyncio => cfg!(unix),
            Self::Uvloop | Self::Winloop | Self::Zuvloop => true,
        }
    }
}

/// Each task runs on an event loop thread while attached to the interpreter.
pub(crate) type LoopTask = Box<dyn FnOnce(Python<'_>, &LoopHandle) -> PyResult<()> + Send>;

/// Holds an event loop and the tasks that Envoy threads hand to it without the
/// GIL.
pub(crate) struct LoopHandle {
    /// The tasks run on this Python event loop.
    pub(crate) loop_: Py<PyAny>,
    state: OnceLock<Option<Py<PyDict>>>,
    eager_task_kwargs: PyOnceLock<Py<PyDict>>,
    queue: Mutex<LoopQueue>,
    waker: Waker,
    /// Limits how many tasks one drain runs before the loop runs its other
    /// callbacks.
    batch_size: usize,
    /// Schedules a callback on the loop from its own thread.
    call_soon: Py<PyAny>,
}

#[derive(Default)]
struct LoopQueue {
    tasks: VecDeque<LoopTask>,
    /// Is set when the loop has been woken and has not drained its tasks yet,
    /// so tasks dispatched together wake it only once.
    wake_pending: bool,
}

/// Describes how Envoy threads wake an event loop that has tasks to run.
enum Waker {
    /// The event loop watches this socket with `add_reader`.
    Socket(WakerSocket),
    /// A thread attaches to the interpreter and schedules draining with
    /// `call_soon_threadsafe`. Event loops that cannot watch a socket use it.
    Thread(mpsc::Sender<WakerMessage>),
}

enum WakerMessage {
    Wake,
    Stop,
}

static NO_STATE: Option<Py<PyDict>> = None;

impl LoopHandle {
    /// Returns the lifespan state to copy into each scope, if lifespan returned
    /// any.
    pub(crate) fn state(&self) -> &Option<Py<PyDict>> {
        self.state.get().unwrap_or(&NO_STATE)
    }

    /// Returns the keyword arguments for starting an eager `asyncio.Task` on
    /// this loop. The loop must be passed explicitly because the task checks
    /// that it is running before starting eagerly.
    pub(crate) fn eager_task_kwargs<'py>(&self, py: Python<'py>) -> PyResult<&Bound<'py, PyDict>> {
        self.eager_task_kwargs
            .get_or_try_init(py, || {
                let kwargs = PyDict::new(py);
                kwargs.set_item("loop", self.loop_.bind(py))?;
                kwargs.set_item("eager_start", true)?;
                Ok::<_, PyErr>(kwargs.unbind())
            })
            .map(|kwargs| kwargs.bind(py))
    }

    /// Queues a task to run on the event loop thread. Callers do not need to
    /// hold the GIL.
    pub(crate) fn dispatch(&self, task: LoopTask) {
        let wake = {
            let mut queue = self.queue.lock().unwrap();
            queue.tasks.push_back(task);
            !std::mem::replace(&mut queue.wake_pending, true)
        };
        if !wake {
            return;
        }
        match &self.waker {
            Waker::Socket(writer) => {
                use std::io::Write as _;
                // A full socket buffer means a wakeup is already pending, so the
                // write can be skipped.
                let _ = (&*writer).write(&[0]);
            }
            Waker::Thread(tx) => {
                let _ = tx.send(WakerMessage::Wake);
            }
        }
    }
}

/// Runs the tasks dispatched to an event loop. The event loop calls it on its
/// own thread.
#[pyclass(module = "_pyvoy.asgi")]
struct LoopDrainer {
    handle: Arc<LoopHandle>,
    reader: Option<WakerSocket>,
}

#[pymethods]
impl LoopDrainer {
    fn __call__(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<()> {
        // Clear the wakeup before taking the tasks, so a task dispatched in
        // between wakes the loop again rather than being missed.
        if let Some(reader) = &slf.reader {
            use std::io::Read as _;
            let mut buf = [0u8; 64];
            while let Ok(n) = (&*reader).read(&mut buf) {
                if n < buf.len() {
                    break;
                }
            }
        }
        let handle = slf.handle.clone();
        let (tasks, more) = {
            let mut queue = handle.queue.lock().unwrap();
            let count = queue.tasks.len().min(handle.batch_size);
            let tasks: Vec<LoopTask> = queue.tasks.drain(..count).collect();
            let more = !queue.tasks.is_empty();
            // The loop stays marked as woken while tasks remain, so producers do
            // not wake it again for tasks that the next drain runs.
            if !more {
                queue.wake_pending = false;
            }
            (tasks, more)
        };
        if more {
            // Running the remaining tasks on a later pass lets the loop run its
            // other callbacks in between, such as resuming requests that
            // already started.
            handle.call_soon.call1(py, (Py::from(slf),))?;
        }
        for task in tasks {
            if let Err(e) = task(py, &handle) {
                report_task_error(py, &handle, e);
            }
        }
        Ok(())
    }
}

/// Sets how many tasks one drain runs when `PYVOY_ASYNC_TASK_BATCH_SIZE` is not
/// set.
const DEFAULT_ASYNC_TASK_BATCH_SIZE: usize = 40;

/// Returns how many tasks one drain runs. `PYVOY_ASYNC_TASK_BATCH_SIZE` overrides
/// the default when it is set to a positive integer.
fn async_task_batch_size() -> usize {
    std::env::var("PYVOY_ASYNC_TASK_BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_ASYNC_TASK_BATCH_SIZE)
}

/// Reports an exception from a task as the event loop reports one from a
/// callback.
fn report_task_error(py: Python<'_>, handle: &LoopHandle, err: PyErr) {
    let report = || {
        let context = PyDict::new(py);
        context.set_item("message", "Exception in pyvoy event loop task")?;
        context.set_item("exception", err.value(py))?;
        handle
            .loop_
            .bind(py)
            .call_method1("call_exception_handler", (context,))?;
        Ok::<_, PyErr>(())
    };
    // The trio adapter has no exception handler.
    if report().is_err() {
        err.display(py);
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
        loop_kind: LoopKind,
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
                        loop_kind,
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
                    loop_kind,
                    constants,
                )?)),
            }),
        }
    }

    /// Gets the event loop to execute a new ASGI application on.
    pub(crate) fn next(&self) -> &LoopHandle {
        match self.inner.as_ref() {
            EventLoopsInner::Single(event_loop) => &event_loop.handle,
            // For now do simple round-robin balancing across event loops. While keeping track
            // of current requests could make sense, it's also hard to know when requests take a
            // while due to I/O, where it's not so bad to have unbalanced but fair load.
            EventLoopsInner::Multiple { loops, next } => {
                let idx = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                &loops[(idx as usize) % loops.len()].handle
            }
        }
    }

    /// Gets the handle of a Python event loop in the pool, which owns futures
    /// created on it.
    pub(crate) fn find(&self, loop_: &Py<PyAny>) -> &LoopHandle {
        let owns = |event_loop: &&EventLoop| event_loop.handle.loop_.as_ptr() == loop_.as_ptr();
        let (found, first) = match self.inner.as_ref() {
            EventLoopsInner::Single(event_loop) => (Some(event_loop).filter(owns), event_loop),
            EventLoopsInner::Multiple { loops, .. } => (loops.iter().find(owns), &loops[0]),
        };
        if let Some(event_loop) = found {
            return &event_loop.handle;
        }
        // pyvoy only completes futures it created on one of its own event loops.
        // Completing a future from another loop's thread is unsafe, so a future
        // from any other loop is reported as a bug before falling back to the
        // first loop.
        debug_assert!(false, "future does not belong to a pyvoy event loop");
        eprintln!("Future does not belong to a pyvoy event loop. This is likely a bug in pyvoy.");
        &first.handle
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
            EventLoopsInner::Single(event_loop) => event_loop.join(),
            EventLoopsInner::Multiple { loops, .. } => {
                for event_loop in loops.iter() {
                    event_loop.join();
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
    handle: Arc<LoopHandle>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    lifespan: Option<Lifespan>,
    constants: Arc<Constants>,
}

impl EventLoop {
    fn new<'py>(
        py: Python<'py>,
        app: &Bound<'py, PyAny>,
        asgi: &Bound<'py, PyDict>,
        enable_lifespan: Option<bool>,
        loop_kind: LoopKind,
        constants: &Arc<Constants>,
    ) -> PyResult<Self> {
        // Import the event loop here rather than on its thread so a missing
        // module fails startup with its traceback.
        let new_event_loop = py
            .import(loop_kind.module())?
            .getattr("new_event_loop")?
            .unbind();
        let (waker, reader, waker_rx) = new_waker(loop_kind)?;
        let batch_size = async_task_batch_size();
        let (tx, rx) = mpsc::channel();
        let rx = SyncReceiver::new(rx);
        let loop_thread = thread::spawn(move || {
            let res: PyResult<()> = Python::attach(|py| {
                let loop_ = new_event_loop.bind(py).call0()?;
                // The trio adapter has no call_soon, and call_soon_threadsafe
                // also works from the loop thread.
                let call_soon = if loop_kind == LoopKind::Trio {
                    loop_.getattr("call_soon_threadsafe")?
                } else {
                    loop_.getattr("call_soon")?
                };
                let handle = Arc::new(LoopHandle {
                    loop_: loop_.clone().unbind(),
                    state: OnceLock::new(),
                    eager_task_kwargs: PyOnceLock::new(),
                    queue: Mutex::default(),
                    waker,
                    batch_size,
                    call_soon: call_soon.unbind(),
                });
                if loop_kind == LoopKind::Trio {
                    // trio finalizes async generators and its worker threads
                    // itself when the run returns.
                    tx.send(handle).unwrap();
                    drop(tx);
                    loop_.call_method0("run_forever")?;
                    return Ok(());
                }

                let asyncio = py.import("asyncio")?;
                asyncio.call_method1("set_event_loop", (&loop_,))?;
                if let Some(reader) = reader {
                    let fd = socket_fd(&reader);
                    let drainer = LoopDrainer {
                        handle: handle.clone(),
                        reader: Some(reader),
                    };
                    loop_.call_method1("add_reader", (fd, drainer))?;
                }
                tx.send(handle).unwrap();
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

        let handle = py.detach(|| rx.recv()).map_err(|e| {
            PyRuntimeError::new_err(format!(
                "Failed to initialize event loop for ASGI executor: {}",
                e
            ))
        })?;
        drop(rx);

        let mut threads = vec![loop_thread];
        if let Some(waker_rx) = waker_rx {
            threads.push(spawn_waker_thread(py, &handle, waker_rx, constants)?);
        }

        let (lifespan, state) = match enable_lifespan {
            Some(false) => (None, None),
            _ => execute_lifespan(
                app,
                asgi,
                handle.loop_.bind(py),
                enable_lifespan.unwrap_or(false),
                loop_kind,
                constants,
            )?,
        };
        let _ = handle.state.set(state);

        Ok(Self {
            handle,
            threads: Mutex::new(threads),
            lifespan,
            constants: constants.clone(),
        })
    }

    fn stop<'py>(&self, py: Python<'py>) -> PyResult<()> {
        let loop_ = self.handle.loop_.bind(py);

        if let Some(lifespan) = self.lifespan.as_ref() {
            lifespan.shutdown(py, loop_);
        }

        if let Waker::Thread(tx) = &self.handle.waker {
            let _ = tx.send(WakerMessage::Stop);
        }
        let stop = loop_.getattr(&self.constants.stop)?;
        loop_.call_method1(&self.constants.call_soon_threadsafe, (stop,))?;
        Ok(())
    }

    fn join(&self) {
        for thread in self.threads.lock().unwrap().drain(..) {
            let _ = thread.join();
        }
    }
}

/// Envoy threads write to one end of a pair of these sockets to wake the event
/// loop, which watches the other end.
#[cfg(unix)]
type WakerSocket = std::os::unix::net::UnixStream;
/// Envoy threads write to one end of a pair of these sockets to wake the event
/// loop, which watches the other end. The standard library has no Unix domain
/// sockets on Windows, so the pair is connected over loopback.
#[cfg(windows)]
type WakerSocket = std::net::TcpStream;

#[cfg(unix)]
fn socket_pair() -> std::io::Result<(WakerSocket, WakerSocket)> {
    std::os::unix::net::UnixStream::pair()
}

#[cfg(windows)]
fn socket_pair() -> std::io::Result<(WakerSocket, WakerSocket)> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let writer = std::net::TcpStream::connect(listener.local_addr()?)?;
    let (reader, _) = listener.accept()?;
    // Each wakeup is a single byte that must be sent right away.
    writer.set_nodelay(true)?;
    Ok((reader, writer))
}

/// Returns the descriptor of a socket to pass to `add_reader`.
#[cfg(unix)]
fn socket_fd(socket: &WakerSocket) -> i64 {
    use std::os::fd::AsRawFd as _;
    i64::from(socket.as_raw_fd())
}

/// Returns the handle of a socket to pass to `add_reader`.
#[cfg(windows)]
fn socket_fd(socket: &WakerSocket) -> u64 {
    use std::os::windows::io::AsRawSocket as _;
    socket.as_raw_socket()
}

/// Creates the waker for an event loop of the given kind. It also returns the
/// socket reader for the event loop to watch, or the receiver for a waker
/// thread.
fn new_waker(
    loop_kind: LoopKind,
) -> PyResult<(
    Waker,
    Option<WakerSocket>,
    Option<mpsc::Receiver<WakerMessage>>,
)> {
    if loop_kind.watches_sockets() {
        let (reader, writer) = socket_pair()
            .and_then(|(reader, writer)| {
                reader.set_nonblocking(true)?;
                writer.set_nonblocking(true)?;
                Ok((reader, writer))
            })
            .map_err(|e| {
                PyRuntimeError::new_err(format!("Failed to create event loop waker: {}", e))
            })?;
        return Ok((Waker::Socket(writer), Some(reader), None));
    }
    let (tx, rx) = mpsc::channel();
    Ok((Waker::Thread(tx), None, Some(rx)))
}

/// Spawns the thread that schedules draining of an event loop that cannot
/// watch a socket.
fn spawn_waker_thread(
    py: Python<'_>,
    handle: &Arc<LoopHandle>,
    rx: mpsc::Receiver<WakerMessage>,
    constants: &Arc<Constants>,
) -> PyResult<JoinHandle<()>> {
    let drainer = Py::new(
        py,
        LoopDrainer {
            handle: handle.clone(),
            reader: None,
        },
    )?;
    let loop_ = handle.loop_.clone_ref(py);
    let constants = constants.clone();
    Ok(thread::spawn(move || {
        while let Ok(WakerMessage::Wake) = rx.recv() {
            Python::attach(|py| {
                // This call fails once the event loop has stopped, when there is
                // nothing left to run.
                let _ = loop_
                    .bind(py)
                    .call_method1(&constants.call_soon_threadsafe, (drainer.bind(py),));
            });
        }
    }))
}
