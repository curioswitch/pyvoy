use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilter;
use envoy_proxy_dynamic_modules_rust_sdk::abi::envoy_dynamic_module_type_attribute_id;
use http::{
    Method,
    uri::{self, Scheme},
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyString},
};

/// Constants used when creating Python objects. These are mostly strings,
/// which PyO3 provides the intern! macro for, but it still has a very small amount
/// of overhead per access, but more importantly forces lazy initialization during
/// request processing. It's not too hard for us to memoize these at startup so
/// we go ahead and do it. Then, usage is just simple ref-counting.
pub(crate) struct Constants {
    // Scheme values
    /// The string "http".
    pub http: Py<PyString>,
    /// The string "https".
    pub https: Py<PyString>,

    // HTTP version strings
    /// The string "http_version".
    pub http_version: Py<PyString>,
    /// The string "1.0".
    pub http_10: Py<PyString>,
    /// The string "1.1".
    pub http_11: Py<PyString>,
    /// The string "2".
    pub http_2: Py<PyString>,
    /// The string "3".
    pub http_3: Py<PyString>,

    // ASGI scope keys
    /// The string "asgi".
    pub asgi: Py<PyString>,
    /// The string "extensions".
    pub extensions: Py<PyString>,
    /// The string "type".
    pub typ: Py<PyString>,
    /// The string "method".
    pub method: Py<PyString>,
    /// The string "path".
    pub path: Py<PyString>,
    /// The string "raw_path".
    pub raw_path: Py<PyString>,
    /// The string "query_string".
    pub query_string: Py<PyString>,
    /// The string "root_path".
    pub root_path: Py<PyString>,
    /// The string "scheme".
    pub scheme: Py<PyString>,
    /// The string "headers".
    pub headers: Py<PyString>,
    /// The string "client".
    pub client: Py<PyString>,
    /// The string "server".
    pub server: Py<PyString>,

    // ASGI event fields
    /// The string "body".
    pub body: Py<PyString>,
    /// The string "more_body".
    pub more_body: Py<PyString>,
    /// The string "http.request".
    pub http_request: Py<PyString>,
    /// The string "http.disconnect".
    pub http_disconnect: Py<PyString>,
    /// The string "status".
    pub status: Py<PyString>,
    /// The string "trailers".
    pub trailers: Py<PyString>,
    /// The string "more_trailers".
    pub more_trailers: Py<PyString>,

    // asyncio methods
    /// The string "asyncio".
    pub asyncio: Py<PyString>,
    /// The string "run_coroutine_threadsafe".
    pub run_coroutine_threadsafe: Py<PyString>,
    /// The string "add_done_callback".
    pub add_done_callback: Py<PyString>,
    /// The string "call_soon_threadsafe".
    pub call_soon_threadsafe: Py<PyString>,
    /// The string "set_result".
    pub set_result: Py<PyString>,
    /// The string "set_exception".
    pub set_exception: Py<PyString>,
    /// The string "create_future".
    pub create_future: Py<PyString>,
    /// The string "result".
    pub result: Py<PyString>,

    // WSGI environ keys
    /// The string "REQUEST_METHOD".
    pub request_method: Py<PyString>,
    /// The string "SCRIPT_NAME".
    pub script_name: Py<PyString>,
    /// The string "PATH_INFO".
    pub path_info: Py<PyString>,
    /// The string "CONTENT_TYPE".
    pub content_type: Py<PyString>,
    /// The string "CONTENT_LENGTH".
    pub content_length: Py<PyString>,
    /// The string "SERVER_NAME".
    pub server_name: Py<PyString>,
    /// The string "SERVER_PORT".
    pub server_port: Py<PyString>,
    /// The string "SERVER_PROTOCOL".
    pub server_protocol: Py<PyString>,
    /// The string "wsgi.version".
    pub wsgi_version: Py<PyString>,
    /// The string "wsgi.url_scheme".
    pub wsgi_url_scheme: Py<PyString>,
    /// The string "wsgi.input".
    pub wsgi_input: Py<PyString>,
    /// The string "wsgi.multithread".
    pub wsgi_multithread: Py<PyString>,
    /// The string "wsgi.multiprocess".
    pub wsgi_multiprocess: Py<PyString>,
    /// The string "wsgi.run_once".
    pub wsgi_run_once: Py<PyString>,

    // Misc
    /// The string "close".
    pub close: Py<PyString>,
}

unsafe impl Sync for Constants {}

impl Constants {
    pub fn new(py: Python<'_>) -> Self {
        Self {
            asgi: PyString::new(py, "asgi").unbind(),
            extensions: PyString::new(py, "extensions").unbind(),
            http: PyString::new(py, "http").unbind(),
            https: PyString::new(py, "https").unbind(),
            method: PyString::new(py, "method").unbind(),
            path: PyString::new(py, "path").unbind(),
            scheme: PyString::new(py, "scheme").unbind(),
            typ: PyString::new(py, "type").unbind(),

            http_version: PyString::new(py, "http_version").unbind(),
            http_10: PyString::new(py, "1.0").unbind(),
            http_11: PyString::new(py, "1.1").unbind(),
            http_2: PyString::new(py, "2").unbind(),
            http_3: PyString::new(py, "3").unbind(),

            raw_path: PyString::new(py, "raw_path").unbind(),
            query_string: PyString::new(py, "query_string").unbind(),
            root_path: PyString::new(py, "root_path").unbind(),
            headers: PyString::new(py, "headers").unbind(),
            client: PyString::new(py, "client").unbind(),
            server: PyString::new(py, "server").unbind(),

            body: PyString::new(py, "body").unbind(),
            more_body: PyString::new(py, "more_body").unbind(),
            http_request: PyString::new(py, "http.request").unbind(),
            http_disconnect: PyString::new(py, "http.disconnect").unbind(),

            asyncio: PyString::new(py, "asyncio").unbind(),
            run_coroutine_threadsafe: PyString::new(py, "run_coroutine_threadsafe").unbind(),
            add_done_callback: PyString::new(py, "add_done_callback").unbind(),
            call_soon_threadsafe: PyString::new(py, "call_soon_threadsafe").unbind(),
            set_result: PyString::new(py, "set_result").unbind(),
            set_exception: PyString::new(py, "set_exception").unbind(),
            create_future: PyString::new(py, "create_future").unbind(),
            result: PyString::new(py, "result").unbind(),

            status: PyString::new(py, "status").unbind(),
            trailers: PyString::new(py, "trailers").unbind(),
            more_trailers: PyString::new(py, "more_trailers").unbind(),

            request_method: PyString::new(py, "REQUEST_METHOD").unbind(),
            script_name: PyString::new(py, "SCRIPT_NAME").unbind(),
            path_info: PyString::new(py, "PATH_INFO").unbind(),
            content_type: PyString::new(py, "CONTENT_TYPE").unbind(),
            content_length: PyString::new(py, "CONTENT_LENGTH").unbind(),
            server_name: PyString::new(py, "SERVER_NAME").unbind(),
            server_port: PyString::new(py, "SERVER_PORT").unbind(),
            server_protocol: PyString::new(py, "SERVER_PROTOCOL").unbind(),
            wsgi_version: PyString::new(py, "wsgi.version").unbind(),
            wsgi_url_scheme: PyString::new(py, "wsgi.url_scheme").unbind(),
            wsgi_input: PyString::new(py, "wsgi.input").unbind(),
            wsgi_multithread: PyString::new(py, "wsgi.multithread").unbind(),
            wsgi_multiprocess: PyString::new(py, "wsgi.multiprocess").unbind(),
            wsgi_run_once: PyString::new(py, "wsgi.run_once").unbind(),

            close: PyString::new(py, "close").unbind(),
        }
    }
}

pub(crate) trait PyDictExt {
    fn set_http_method<'py>(
        &self,
        py: Python<'py>,
        key: &Bound<'py, PyString>,
        method: &http::Method,
    ) -> PyResult<()>;

    fn set_http_scheme<'py>(
        &self,
        py: Python<'py>,
        constants: &Constants,
        key: &Bound<'py, PyString>,
        scheme: &uri::Scheme,
    ) -> PyResult<()>;

    fn set_http_version<'py>(
        &self,
        py: Python<'py>,
        constants: &Constants,
        key: &Bound<'py, PyString>,
        version: &http::Version,
    ) -> PyResult<()>;
}

impl PyDictExt for Bound<'_, PyDict> {
    fn set_http_method<'py>(
        &self,
        _py: Python<'py>,
        key: &Bound<'py, PyString>,
        method: &http::Method,
    ) -> PyResult<()> {
        self.set_item(key, method.as_str())?;
        Ok(())
    }

    fn set_http_scheme<'py>(
        &self,
        py: Python<'py>,
        constants: &Constants,
        key: &Bound<'py, PyString>,
        scheme: &uri::Scheme,
    ) -> PyResult<()> {
        if *scheme == uri::Scheme::HTTP {
            self.set_item(key, constants.http.bind(py))?;
        } else if *scheme == uri::Scheme::HTTPS {
            self.set_item(key, constants.https.bind(py))?;
        } else {
            self.set_item(key, scheme.as_str())?;
        }
        Ok(())
    }

    fn set_http_version<'py>(
        &self,
        py: Python<'py>,
        constants: &Constants,
        key: &Bound<'py, PyString>,
        version: &http::Version,
    ) -> PyResult<()> {
        self.set_item(
            key,
            match version {
                &http::Version::HTTP_10 => constants.http_10.bind(py),
                &http::Version::HTTP_11 => constants.http_11.bind(py),
                &http::Version::HTTP_2 => constants.http_2.bind(py),
                &http::Version::HTTP_3 => constants.http_3.bind(py),
                _ => constants.http_11.bind(py),
            },
        )?;
        Ok(())
    }
}

fn is_pseudoheader(http_version: &http::Version, name: &[u8]) -> bool {
    http_version >= &http::Version::HTTP_2
        && matches!(name, b":method" | b":scheme" | b":authority" | b":path")
}

pub(crate) struct Scope {
    pub http_version: http::Version,
    pub method: Method,
    pub scheme: Scheme,
    pub raw_path: Box<[u8]>,
    pub query_string: Box<[u8]>,
    pub headers: Vec<(Box<[u8]>, Box<[u8]>)>,
    pub client: Option<(Box<str>, i64)>,
    pub server: Option<(Box<str>, i64)>,
}

pub(crate) fn new_scope<EHF: EnvoyHttpFilter>(envoy_filter: &EHF) -> Scope {
    let http_version = match envoy_filter
        .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestProtocol)
    {
        Some(v) => match v.as_slice() {
            b"HTTP/1.0" => http::Version::HTTP_10,
            b"HTTP/1.1" => http::Version::HTTP_11,
            b"HTTP/2" => http::Version::HTTP_2,
            b"HTTP/3" => http::Version::HTTP_3,
            _ => http::Version::HTTP_11,
        },
        None => http::Version::HTTP_11,
    };
    let method = envoy_filter
        .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestMethod)
        .and_then(|v| http::Method::from_bytes(v.as_slice()).ok())
        .unwrap_or(http::Method::GET);

    let scheme = match envoy_filter
        .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestScheme)
    {
        Some(v) => match v.as_slice() {
            b"http" => uri::Scheme::HTTP,
            b"https" => uri::Scheme::HTTPS,
            _ => uri::Scheme::HTTP,
        },
        None => uri::Scheme::HTTP,
    };

    let raw_path: Box<[u8]> = match envoy_filter
        .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestUrlPath)
    {
        Some(v) => Box::from(v.as_slice()),
        None => Box::from(&b"/"[..]),
    };

    let query_string = match envoy_filter
        .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestQuery)
    {
        Some(v) => Box::from(v.as_slice()),
        None => Box::from(&b""[..]),
    };

    let headers = envoy_filter
        .get_request_headers()
        .iter()
        .filter_map(|(k, v)| {
            if !is_pseudoheader(&http_version, k.as_slice()) {
                Some((Box::from(k.as_slice()), Box::from(v.as_slice())))
            } else {
                None
            }
        })
        .collect();

    let client = get_address(
        envoy_filter,
        envoy_dynamic_module_type_attribute_id::SourceAddress,
        envoy_dynamic_module_type_attribute_id::SourcePort,
    );
    let server = get_address(
        envoy_filter,
        envoy_dynamic_module_type_attribute_id::DestinationAddress,
        envoy_dynamic_module_type_attribute_id::DestinationPort,
    );

    Scope {
        http_version,
        method,
        scheme,
        raw_path,
        query_string,
        headers,
        client,
        server,
    }
}

fn get_address<EHF: EnvoyHttpFilter>(
    envoy_filter: &EHF,
    address_attr_id: envoy_dynamic_module_type_attribute_id,
    port_attr_id: envoy_dynamic_module_type_attribute_id,
) -> Option<(Box<str>, i64)> {
    match (
        envoy_filter.get_attribute_string(address_attr_id),
        envoy_filter.get_attribute_int(port_attr_id),
    ) {
        (Some(host), Some(port)) => {
            let mut host = host.as_slice();
            if let Some(colon_idx) = host.iter().position(|&c| c == b':') {
                host = &host[..colon_idx];
            }
            Some((Box::from(str::from_utf8(host).unwrap_or("")), port))
        }
        _ => None,
    }
}

pub(crate) fn has_request_body<EHF: EnvoyHttpFilter>(envoy_filter: &mut EHF) -> bool {
    if let Some(buffers) = envoy_filter.get_request_body() {
        for buffer in buffers {
            if !buffer.as_slice().is_empty() {
                return true;
            }
        }
    }
    false
}
