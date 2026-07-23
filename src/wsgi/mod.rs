/// The WSGI filter implementation.
pub(crate) mod filter;
/// An elastic thread pool for blocking work.
mod pool;
/// The Python side of the WSGI handler.
mod python;
/// Utilities for sending the WSGI response.
mod response;
/// The pyqwest transport implementation.
mod transport;
/// Request and response event types.
mod types;

pub(crate) fn register_py_modules(py: pyo3::Python<'_>) -> pyo3::PyResult<()> {
    transport::register_py_module(py)
}
