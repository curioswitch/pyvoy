//! Completes event loop futures, skipping already completed
//! (generally canceled) futures.``

use std::sync::Arc;

use pyo3::prelude::*;

use crate::types::Constants;

/// Completes the future with the result unless it is already done.
pub(crate) fn set_result<'py>(
    future: &Bound<'py, PyAny>,
    constants: &Constants,
    result: impl IntoPyObject<'py>,
) -> PyResult<()> {
    if is_done(future, constants)? {
        return Ok(());
    }
    future.call_method1(&constants.set_result, (result,))?;
    Ok(())
}

/// Fails the future with the exception unless it is already done.
pub(crate) fn set_exception<'py>(
    future: &Bound<'py, PyAny>,
    constants: &Constants,
    exception: impl IntoPyObject<'py>,
) -> PyResult<()> {
    if is_done(future, constants)? {
        return Ok(());
    }
    future.call_method1(&constants.set_exception, (exception,))?;
    Ok(())
}

/// Whether the future is already done, e.g. cancelled by the application.
pub(crate) fn is_done(future: &Bound<'_, PyAny>, constants: &Constants) -> PyResult<bool> {
    future.call_method0(&constants.done)?.is_truthy()
}

/// Completes a future with a result when called, for scheduling with
/// `call_soon_threadsafe` from outside the event loop.
#[pyclass(module = "_pyvoy.asgi")]
pub(crate) struct SetResultCallback {
    future: Py<PyAny>,
    result: Py<PyAny>,
    constants: Arc<Constants>,
}

impl SetResultCallback {
    pub(crate) fn new(future: Py<PyAny>, result: Py<PyAny>, constants: Arc<Constants>) -> Self {
        Self {
            future,
            result,
            constants,
        }
    }
}

#[pymethods]
impl SetResultCallback {
    fn __call__(&self, py: Python<'_>) -> PyResult<()> {
        set_result(self.future.bind(py), &self.constants, self.result.bind(py))
    }
}
