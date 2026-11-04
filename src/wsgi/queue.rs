use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, Thread};

/// Holds work for a pool of threads and wakes the most recently idle thread
/// first.
///
/// Waking a thread that only just went idle is much cheaper than waking the
/// thread that has been idle the longest, which is what a first-in, first-out
/// channel does. It also keeps work on the few threads that are needed rather
/// than rotating through the whole pool.
pub(crate) struct WorkQueue<T> {
    state: Mutex<State<T>>,
}

struct State<T> {
    items: VecDeque<T>,
    /// Lists the threads waiting for an item, with the most recently idle last.
    idle: Vec<Arc<Parker>>,
    closed: bool,
}

/// Represents a thread waiting for an item. It is listed as idle only while it
/// waits.
struct Parker {
    thread: Thread,
    notified: AtomicBool,
}

impl Parker {
    fn notify(&self) {
        self.notified.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

thread_local! {
    static PARKER: Arc<Parker> = Arc::new(Parker {
        thread: thread::current(),
        notified: AtomicBool::new(false),
    });
}

impl<T> WorkQueue<T> {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(State {
                items: VecDeque::new(),
                idle: Vec::new(),
                closed: false,
            }),
        }
    }

    /// Adds an item, waking the most recently idle thread if there is one.
    pub(crate) fn push(&self, item: T) {
        let parker = {
            let mut state = self.state.lock().unwrap();
            state.items.push_back(item);
            state.idle.pop()
        };
        if let Some(parker) = parker {
            parker.notify();
        }
    }

    /// Takes the next item, blocking until there is one. Returns `None` once
    /// the queue is closed and has no items left.
    pub(crate) fn pop(&self) -> Option<T> {
        PARKER.with(|parker| {
            loop {
                {
                    let mut state = self.state.lock().unwrap();
                    if let Some(item) = state.items.pop_front() {
                        return Some(item);
                    }
                    if state.closed {
                        return None;
                    }
                    parker.notified.store(false, Ordering::Relaxed);
                    state.idle.push(parker.clone());
                }
                // park can return spuriously, so wait until notified, which
                // also means this thread is no longer listed as idle.
                while !parker.notified.load(Ordering::Acquire) {
                    thread::park();
                }
            }
        })
    }

    /// Closes the queue, waking all idle threads to drain the remaining items.
    pub(crate) fn close(&self) {
        let idle = {
            let mut state = self.state.lock().unwrap();
            state.closed = true;
            std::mem::take(&mut state.idle)
        };
        for parker in idle {
            parker.notify();
        }
    }

    #[cfg(test)]
    fn idle_threads(&self) -> usize {
        self.state.lock().unwrap().idle.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn wait_for_idle(queue: &WorkQueue<u32>, count: usize) {
        for _ in 0..1000 {
            if queue.idle_threads() == count {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("expected {count} idle threads");
    }

    #[test]
    fn items_are_taken_in_order() {
        let queue = WorkQueue::new();
        queue.push(1);
        queue.push(2);
        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(2));
    }

    #[test]
    fn wakes_most_recently_idle_thread() {
        let queue = Arc::new(WorkQueue::new());
        let (tx, rx) = mpsc::channel();
        let mut handles = Vec::new();
        for name in ["first", "second"] {
            let worker_queue = queue.clone();
            let tx = tx.clone();
            handles.push(thread::spawn(move || {
                while let Some(item) = worker_queue.pop() {
                    tx.send((name, item)).unwrap();
                }
            }));
            wait_for_idle(&queue, handles.len());
        }

        queue.push(1);
        assert_eq!(rx.recv().unwrap(), ("second", 1));
        wait_for_idle(&queue, 2);
        queue.push(2);
        assert_eq!(rx.recv().unwrap(), ("second", 2));

        queue.close();
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn close_drains_remaining_items() {
        let queue = Arc::new(WorkQueue::new());
        let worker = {
            let queue = queue.clone();
            thread::spawn(move || {
                let mut items = Vec::new();
                while let Some(item) = queue.pop() {
                    items.push(item);
                }
                items
            })
        };
        wait_for_idle(&queue, 1);
        queue.push(1);
        queue.push(2);
        queue.close();
        assert_eq!(worker.join().unwrap(), vec![1, 2]);
    }
}
