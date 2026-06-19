use pyo3::{Bound, PyAny, call::PyCallArgs, types::PyAnyMethods as _};

pub(crate) struct RunOnDrop<'py, A>(pub(crate) Bound<'py, PyAny>, pub(crate) Option<A>)
where
    A: PyCallArgs<'py>;

impl<'py, A> Drop for RunOnDrop<'py, A>
where
    A: PyCallArgs<'py>,
{
    fn drop(&mut self) {
        if let Some(args) = self.1.take() {
            let _ = self.0.call1(args);
        } else {
            let _ = self.0.call0();
        }
    }
}
