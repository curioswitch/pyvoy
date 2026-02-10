use http::{HeaderName, HeaderValue};
use pyo3::{
    Bound, PyResult, Python,
    exceptions::PyValueError,
    types::{PyAnyMethods as _, PyBytes, PyBytesMethods as _, PyDict, PyDictMethods as _},
};

use crate::types::Constants;

/// Converts headers from Python to Rust.
pub(crate) fn extract_headers_from_event<'py>(
    _py: Python<'py>,
    constants: &Constants,
    event: &Bound<'py, PyDict>,
) -> PyResult<Vec<(HeaderName, HeaderValue)>> {
    match event.get_item(&constants.headers)? {
        Some(v) => {
            let cap = v.len().unwrap_or(0);
            let mut headers = Vec::with_capacity(cap);
            for item in v.try_iter()? {
                let tuple = item?;
                let key_item = tuple.get_item(0)?;
                let value_item = tuple.get_item(1)?;
                let key_bytes = key_item.cast::<PyBytes>()?;
                let value_bytes = value_item.cast::<PyBytes>()?;
                headers.push((
                    HeaderName::from_bytes(key_bytes.as_bytes())
                        .map_err(|e| PyValueError::new_err(e.to_string()))?,
                    HeaderValue::from_bytes(value_bytes.as_bytes())
                        .map_err(|e| PyValueError::new_err(e.to_string()))?,
                ));
            }
            Ok(headers)
        }
        None => Ok(Vec::default()),
    }
}
