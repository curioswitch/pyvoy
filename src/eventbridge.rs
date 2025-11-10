use std::sync::{Arc, Mutex};

enum Inner<T> {
    Closed,
    Empty,
    Single(T),
    Multiple(Vec<T>),
}

/// An optimized event bridge between Envoy and Python.
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

    /// Drains all events on the bridge, calling `f` for each.
    ///
    /// We only lock the mutex to drain and then `send` calls can continue while
    /// processing.
    pub(crate) fn process(&self, mut f: impl FnMut(T)) {
        let mut inner = self.inner.lock().unwrap();
        match std::mem::replace(&mut *inner, Inner::Empty) {
            Inner::Closed | Inner::Empty => {}
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

    /// Closes the bridge. Further `send` calls will fail.
    pub(crate) fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        *inner = Inner::Closed;
    }
}
