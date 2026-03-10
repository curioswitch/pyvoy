use std::sync::Arc;

use crate::{
    headernames::HeaderNameExt as _,
    types::{Constants, PyDictExt as _, Scope},
};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyString},
};

pub(crate) fn new_scope_dict<'py>(
    py: Python<'py>,
    scope: Scope,
    typ: &Py<PyString>,
    asgi: &Py<PyDict>,
    extensions: &Py<PyDict>,
    state: &Option<Py<PyDict>>,
    constants: &Arc<Constants>,
) -> PyResult<Bound<'py, PyDict>> {
    let scope_dict = PyDict::new(py);
    scope_dict.set_item(&constants.typ, typ)?;
    scope_dict.set_item(&constants.asgi, asgi)?;

    let mut extensions = extensions.bind(py).clone();
    if let Some(tls_info) = scope.tls_info {
        extensions = extensions.copy()?;
        let tls_dict = PyDict::new(py);
        tls_dict.set_item(&constants.tls_version, tls_info.tls_version)?;
        if let Some(client_cert_name) = tls_info.client_cert_name {
            tls_dict.set_item(
                &constants.client_cert_name,
                PyString::new(py, &client_cert_name),
            )?;
        } else {
            tls_dict.set_item(&constants.client_cert_name, py.None())?;
        }
        extensions.set_item(&constants.tls, tls_dict)?;
    }
    scope_dict.set_item(&constants.extensions, extensions)?;

    if let Some(state) = state {
        scope_dict.set_item(&constants.state, state)?;
    }

    scope_dict.set_http_version(constants, &scope.http_version)?;
    scope_dict.set_http_method(constants, &constants.method, &scope.method)?;
    scope_dict.set_http_scheme(constants, &constants.scheme, &scope.scheme)?;

    let raw_path: &[u8] = if let Some(query_idx) = scope.raw_path.iter().position(|&b| b == b'?') {
        scope_dict.set_item(
            &constants.query_string,
            PyBytes::new(py, &scope.raw_path[query_idx + 1..]),
        )?;
        &scope.raw_path[..query_idx]
    } else {
        scope_dict.set_item(&constants.query_string, &constants.empty_bytes)?;
        &scope.raw_path
    };

    let decoded_path = urlencoding::decode_binary(raw_path);
    scope_dict.set_item(&constants.path, PyString::from_bytes(py, &decoded_path)?)?;
    scope_dict.set_item(&constants.raw_path, PyBytes::new(py, raw_path))?;
    scope_dict.set_item(&constants.root_path, &constants.root_path_value)?;
    let headers = PyList::new(
        py,
        scope.headers.iter().map(|(k, v)| {
            (
                k.to_asgi_bytes(py, constants),
                PyBytes::new(py, v.as_bytes()),
            )
        }),
    )?;
    scope_dict.set_item(&constants.headers, headers)?;
    scope_dict.set_item(
        &constants.client,
        scope.client.map(|(a, p)| (PyString::new(py, &a[..]), p)),
    )?;
    scope_dict.set_item(
        &constants.server,
        scope.server.map(|(a, p)| (PyString::new(py, &a[..]), p)),
    )?;

    Ok(scope_dict)
}
