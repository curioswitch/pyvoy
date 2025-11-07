use pyo3::{IntoPyObjectExt, exceptions::PyStopIteration, prelude::*};

/// An awaitable that returns `None` when awaited.
#[pyclass]
pub(crate) struct EmptyAwaitable;

impl EmptyAwaitable {
    /// Creates a new [`EmptyAwaitable`].
    pub(crate) fn new_py<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Self {}.into_bound_py_any(py)
    }
}

#[pymethods]
impl EmptyAwaitable {
    fn __await__<'py>(slf: PyRef<'py, Self>) -> PyRef<'py, Self> {
        slf
    }

    fn __next__(&self) -> Option<()> {
        None
    }
}

/// An awaitable that returns the given value when awaited.
#[pyclass]
pub(crate) struct ValueAwaitable {
    value: Option<Py<PyAny>>,
}

impl ValueAwaitable {
    /// Creates a new [`ValueAwaitable`].
    pub(crate) fn new_py<'py>(py: Python<'py>, value: Py<PyAny>) -> PyResult<Bound<'py, PyAny>> {
        Self { value: Some(value) }.into_bound_py_any(py)
    }
}

#[pymethods]
impl ValueAwaitable {
    fn __await__<'py>(slf: PyRef<'py, Self>) -> PyRef<'py, Self> {
        slf
    }

    fn __next__(&mut self) -> PyResult<Py<PyAny>> {
        if let Some(value) = self.value.take() {
            Err(PyStopIteration::new_err(value))
        } else {
            // Shouldn't happen in practice.
            Err(PyStopIteration::new_err(()))
        }
    }
}

/// An awaitable that raises the given error when awaited.
#[pyclass]
pub(crate) struct ErrorAwaitable {
    error: Option<PyErr>,
}

impl ErrorAwaitable {
    /// Creates a new [`ErrorAwaitable`].
    pub(crate) fn new_py<'py>(py: Python<'py>, error: PyErr) -> PyResult<Bound<'py, PyAny>> {
        Self { error: Some(error) }.into_bound_py_any(py)
    }
}

#[pymethods]
impl ErrorAwaitable {
    fn __await__<'py>(slf: PyRef<'py, Self>) -> PyRef<'py, Self> {
        slf
    }

    fn __next__(&mut self) -> PyResult<()> {
        if let Some(error) = self.error.take() {
            Err(error)
        } else {
            // Shouldn't happen in practice.
            Ok(())
        }
    }
}
