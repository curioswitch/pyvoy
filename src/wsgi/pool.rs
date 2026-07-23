use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender, TrySendError, select};

/// How long an idle worker waits for a job before exiting, letting the pool
/// shrink back to zero threads when there is no work.
const KEEP_ALIVE: Duration = Duration::from_secs(10);

type Job = Box<dyn FnOnce() + Send + 'static>;

struct Inner {
    /// Rendezvous channel sender for jobs submitted to the pool.
    job_tx: Sender<Job>,
    /// Channel receiver for jobs submitted to the pool. A worker blocks on this channel
    /// waiting for a job to run.
    job_rx: Receiver<Job>,
    /// Disconnected when the pool shuts down, waking idle workers immediately.
    shutdown_rx: Receiver<()>,
    /// Whether to shutdown.
    shutdown: AtomicBool,
    /// The number of currently running workers. Used to wait for all workers to exit on shutdown.
    workers: Mutex<usize>,
    /// Used to wake the shutdown thread when all workers have exited.
    idle: Condvar,
}

/// An elastic thread pool for blocking work, modeled on `tokio::task::spawn_blocking`:
/// threads are spawned on demand and reaped after an idle period, so the pool uses no
/// threads when idle.
pub(crate) struct BlockingPool {
    inner: Arc<Inner>,
    shutdown_tx: Option<Sender<()>>,
}

/// A handle used to submit work to a [`BlockingPool`]. Cloneable and safe to keep alive
/// past the pool's shutdown, since it does not hold the pool's shutdown signal.
#[derive(Clone)]
pub(crate) struct PoolHandle {
    inner: Arc<Inner>,
}

impl BlockingPool {
    pub(crate) fn new() -> Self {
        let (job_tx, job_rx) = crossbeam_channel::bounded(0);
        let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(0);
        Self {
            inner: Arc::new(Inner {
                job_tx,
                job_rx,
                shutdown_rx,
                shutdown: AtomicBool::new(false),
                workers: Mutex::new(0),
                idle: Condvar::new(),
            }),
            shutdown_tx: Some(shutdown_tx),
        }
    }

    pub(crate) fn handle(&self) -> PoolHandle {
        PoolHandle {
            inner: self.inner.clone(),
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.inner.shutdown.store(true, Ordering::Relaxed);
        // Dropping the only shutdown sender disconnects `shutdown_rx`, waking every
        // idle worker; busy workers observe the disconnect when they finish a job.
        drop(self.shutdown_tx.take());
        let mut workers = self.inner.workers.lock().unwrap();
        while *workers > 0 {
            workers = self.inner.idle.wait(workers).unwrap();
        }
    }
}

impl PoolHandle {
    pub(crate) fn execute<F: FnOnce() + Send + 'static>(&self, job: F) {
        if self.inner.shutdown.load(Ordering::Relaxed) {
            return;
        }
        match self.inner.job_tx.try_send(Box::new(job)) {
            // An idle worker picked up the job.
            Ok(()) => {}
            // No idle worker; spawn one to run the job and remain for reuse.
            Err(TrySendError::Full(job)) => {
                *self.inner.workers.lock().unwrap() += 1;
                let inner = self.inner.clone();
                thread::spawn(move || run_worker(inner, job));
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

fn run_worker(inner: Arc<Inner>, initial: Job) {
    initial();
    loop {
        select! {
            recv(inner.job_rx) -> job => match job {
                Ok(job) => job(),
                Err(_) => break,
            },
            recv(inner.shutdown_rx) -> _ => break,
            default(KEEP_ALIVE) => break,
        }
    }
    let mut workers = inner.workers.lock().unwrap();
    *workers -= 1;
    if *workers == 0 {
        inner.idle.notify_all();
    }
}
