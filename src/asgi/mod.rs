/// The ASGI filter implementation.
pub(crate) mod filter;

/// The Python side of the ASGI handler.
mod python;
mod shared;
/// The pyqwest transport implementation.
mod transport;
/// The ASGI websocket filter.
pub(crate) mod websocket;

pub(crate) fn register_py_modules(py: pyo3::Python<'_>) -> pyo3::PyResult<()> {
    transport::register_py_module(py)
}
