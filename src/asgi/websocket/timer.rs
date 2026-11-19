use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    sync::{Mutex, mpsc},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilterScheduler;

/// A request to commit an event on a scheduler after a delay.
type Request = (Duration, Box<dyn EnvoyHttpFilterScheduler>, u64);

struct Entry {
    deadline: Instant,
    /// Breaks deadline ties so entries never compare their schedulers.
    seq: u64,
    scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    event_id: u64,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        (self.deadline, self.seq) == (other.deadline, other.seq)
    }
}

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.deadline, self.seq).cmp(&(other.deadline, other.seq))
    }
}

/// Commits scheduler events after a delay. HTTP filters have no timers in the
/// dynamic module ABI, so one thread per filter config serves every stream.
///
/// A committed event for a stream that has since completed is dropped by
/// Envoy, so entries are never cancelled.
pub(super) struct Timer {
    sender: Option<Mutex<mpsc::Sender<Request>>>,
    thread: Option<JoinHandle<()>>,
}

impl Timer {
    pub(super) fn new() -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<Request>();
        let thread = std::thread::Builder::new()
            .name("pyvoy-ws-timer".into())
            .spawn(move || run(receiver))?;
        Ok(Self {
            sender: Some(Mutex::new(sender)),
            thread: Some(thread),
        })
    }

    /// Commits `event_id` on the scheduler once `delay` elapses.
    pub(super) fn schedule(
        &self,
        delay: Duration,
        scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        event_id: u64,
    ) {
        if let Some(sender) = &self.sender {
            // Only fails once the thread has exited during shutdown.
            let _ = sender.lock().unwrap().send((delay, scheduler, event_id));
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        // Disconnecting the channel stops the thread.
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(receiver: mpsc::Receiver<Request>) {
    let mut entries: BinaryHeap<Reverse<Entry>> = BinaryHeap::new();
    let mut seq = 0u64;
    loop {
        let now = Instant::now();
        while entries
            .peek()
            .is_some_and(|Reverse(entry)| entry.deadline <= now)
        {
            let Reverse(entry) = entries.pop().unwrap();
            entry.scheduler.commit(entry.event_id);
        }
        let received = match entries.peek() {
            Some(Reverse(entry)) => {
                match receiver.recv_timeout(entry.deadline.saturating_duration_since(now)) {
                    Ok(received) => received,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
            None => match receiver.recv() {
                Ok(received) => received,
                Err(_) => return,
            },
        };
        let (delay, scheduler, event_id) = received;
        seq += 1;
        entries.push(Reverse(Entry {
            deadline: Instant::now() + delay,
            seq,
            scheduler,
            event_id,
        }));
    }
}
