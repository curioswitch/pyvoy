pub(super) mod app;
pub(super) mod awaitable;
pub(super) mod eventloop;
pub(super) mod headers;
mod lifespan;
pub(super) mod scope;

/// Holds the event loops created by an executor to join their threads.
pub(crate) struct ExecutorHandles {
    pub(crate) loops: eventloop::EventLoops,
}

impl ExecutorHandles {
    pub(crate) fn join(self) {
        self.loops.join();
    }
}
