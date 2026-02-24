use std::thread::JoinHandle;

pub(super) mod app;
pub(super) mod awaitable;
pub(super) mod eventloop;
pub(super) mod headers;
mod lifespan;
pub(super) mod scope;

/// Holds [`JoinHandle`] for threads created by an executor.
pub(crate) struct ExecutorHandles {
    pub(crate) loops: eventloop::EventLoops,
    pub(crate) gil_handle: JoinHandle<()>,
}

impl ExecutorHandles {
    pub(crate) fn join(self) {
        self.loops.join();
        let _ = self.gil_handle.join();
    }
}

pub(crate) fn get_gil_batch_size() -> usize {
    std::env::var("PYVOY_GIL_BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40)
}
