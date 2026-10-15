use std::sync::{Arc, Mutex};

enum Inner<T> {
    Closed,
    Empty,
    Single(T),
    Multiple(Vec<T>),
}

/// An optimized event bridge from Python back to Envoy.
///
/// Because of the design of the Envoy SDK, we cannot directly pass an event to
/// an Envoy callback - we always push the event and then schedule Envoy to process
/// it. This means channels, while working as a simple mpsc queue, are overkill.
/// We instead use a simple mutex-guarded queue. Because any well-behaved ASGI/WSGI
/// application will only produce one event at a time, we optimize for that case
/// to avoid unnecessary allocations while supporting multiple events if for some reason
/// the app issues them.
pub(crate) struct EventBridge<T> {
    inner: Arc<Mutex<Inner<T>>>,
}

impl<T> Clone for EventBridge<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> EventBridge<T> {
    /// Creates a new empty [`EventBridge`].
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::Empty)),
        }
    }

    /// Sends an event on the bridge.
    pub(crate) fn send(&self, event: T) -> Result<(), T> {
        let mut inner = self.inner.lock().unwrap();
        match *inner {
            Inner::Closed => {
                return Err(event);
            }
            Inner::Empty => {
                *inner = Inner::Single(event);
            }
            Inner::Single(_) => {
                let Inner::Single(first) = std::mem::replace(&mut *inner, Inner::Empty) else {
                    unreachable!()
                };
                *inner = Inner::Multiple(vec![first, event]);
            }
            Inner::Multiple(ref mut events) => {
                events.push(event);
            }
        }
        Ok(())
    }

    pub(crate) fn is_empty(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        matches!(*inner, Inner::Empty | Inner::Closed)
    }

    /// Drains all events on the bridge, calling `f` for each.
    ///
    /// We only lock the mutex to drain and then `send` calls can continue while
    /// processing.
    pub(crate) fn process(&self, mut f: impl FnMut(T)) {
        let mut inner = self.inner.lock().unwrap();
        match std::mem::replace(&mut *inner, Inner::Empty) {
            // Draining must not reopen a closed bridge: on_scheduled can run
            // after on_stream_complete closed it.
            Inner::Closed => *inner = Inner::Closed,
            Inner::Empty => {}
            Inner::Single(event) => {
                drop(inner);
                f(event);
            }
            Inner::Multiple(events) => {
                drop(inner);
                for event in events {
                    f(event);
                }
            }
        }
    }

    /// Gets a single event from the bridge.
    pub(crate) fn get(&self) -> Option<T> {
        let mut inner = self.inner.lock().unwrap();
        match std::mem::replace(&mut *inner, Inner::Empty) {
            Inner::Closed => {
                *inner = Inner::Closed;
                None
            }
            Inner::Empty => None,
            Inner::Single(event) => Some(event),
            Inner::Multiple(mut events) => {
                let event = events.remove(0);
                if !events.is_empty() {
                    *inner = Inner::Multiple(events);
                }
                Some(event)
            }
        }
    }

    /// Closes the bridge. Further `send` calls will fail.
    pub(crate) fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        *inner = Inner::Closed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_keeps_closed_bridge_closed() {
        let bridge = EventBridge::<u32>::new();
        bridge.close();
        bridge.process(|_| panic!("closed bridge has no events"));
        assert_eq!(bridge.send(1), Err(1));
    }

    #[test]
    fn get_keeps_closed_bridge_closed() {
        let bridge = EventBridge::<u32>::new();
        bridge.close();
        assert!(bridge.get().is_none());
        assert_eq!(bridge.send(1), Err(1));
    }

    #[test]
    fn process_drains_in_order() {
        let bridge = EventBridge::<u32>::new();
        bridge.send(1).unwrap();
        bridge.send(2).unwrap();
        let mut seen = Vec::new();
        bridge.process(|event| seen.push(event));
        assert_eq!(seen, vec![1, 2]);
        assert!(bridge.is_empty());
        bridge.send(3).unwrap();
        assert_eq!(bridge.get(), Some(3));
    }
}
