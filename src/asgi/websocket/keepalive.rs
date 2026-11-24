use std::time::{Duration, Instant};

/// Tracks whether the client answers keepalive pings. The timer can't cancel
/// entries, so at most one timeout is scheduled at a time and it reschedules
/// itsel.
#[derive(Default)]
pub(super) struct PingTracker {
    /// When the oldest ping the client has not answered was sent. Any data from
    /// the client answers it.
    unanswered_since: Option<Instant>,
    timeout_scheduled: bool,
}

pub(super) enum TimeoutAction {
    /// The client answered, so the next ping schedules a new timeout.
    None,
    /// The client answered and was pinged again, so wait for that ping.
    Reschedule(Duration),
    /// The client did not answer in time.
    Close,
}

impl PingTracker {
    pub(super) fn on_client_data(&mut self) {
        self.unanswered_since = None;
    }

    /// Records a ping sent at `now`, returning whether to schedule a timeout.
    pub(super) fn on_ping(&mut self, now: Instant) -> bool {
        self.unanswered_since.get_or_insert(now);
        !std::mem::replace(&mut self.timeout_scheduled, true)
    }

    pub(super) fn on_timeout(&mut self, now: Instant, timeout: Duration) -> TimeoutAction {
        self.timeout_scheduled = false;
        let Some(sent) = self.unanswered_since else {
            return TimeoutAction::None;
        };
        let elapsed = now.saturating_duration_since(sent);
        if elapsed >= timeout {
            return TimeoutAction::Close;
        }
        self.timeout_scheduled = true;
        TimeoutAction::Reschedule(timeout - elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVAL: Duration = Duration::from_secs(1);
    const TIMEOUT: Duration = Duration::from_secs(10);

    /// Pings every second for 100 seconds, with the client answering the pings
    /// sent up to `answered_until` seconds. Returns how many timeouts were
    /// scheduled, at most one at a time, and when the client was closed.
    fn simulate(answered_until: u32) -> (u32, Option<u32>) {
        let start = Instant::now();
        let mut tracker = PingTracker::default();
        let mut deadline: Option<Instant> = None;
        let mut scheduled = 0;
        for second in 1..=100 {
            let now = start + INTERVAL * second;
            if let Some(due) = deadline
                && due <= now
            {
                deadline = None;
                match tracker.on_timeout(now, TIMEOUT) {
                    TimeoutAction::None => {}
                    TimeoutAction::Reschedule(delay) => {
                        scheduled += 1;
                        deadline = Some(now + delay);
                    }
                    TimeoutAction::Close => return (scheduled, Some(second)),
                }
            }
            if tracker.on_ping(now) {
                assert!(deadline.is_none(), "a timeout is already scheduled");
                scheduled += 1;
                deadline = Some(now + TIMEOUT);
            }
            if second <= answered_until {
                tracker.on_client_data();
            }
        }
        (scheduled, None)
    }

    #[test]
    fn answered_pings_schedule_one_timeout_per_timeout_period() {
        assert_eq!(simulate(100), (10, None));
    }

    #[test]
    fn unanswered_ping_closes_after_timeout() {
        // The first unanswered ping is sent at 6s.
        assert_eq!(simulate(5).1, Some(16));
        assert_eq!(simulate(0).1, Some(11));
    }
}
