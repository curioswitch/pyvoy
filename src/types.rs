use std::sync::mpsc::Receiver;

use crate::envoy::*;
use crate::headernames::{ASGIHeaderNameConstants, WSGIHeaderNameConstants};
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilter;
use envoy_proxy_dynamic_modules_rust_sdk::abi::envoy_dynamic_module_type_attribute_id;
use http::{
    HeaderName, HeaderValue, Method,
    uri::{self, Scheme},
};
use pyo3::{
    IntoPyObjectExt, create_exception,
    exceptions::PyOSError,
    prelude::*,
    types::{PyBytes, PyDict, PyNone, PyString},
};

create_exception!(pyvoy, ClientDisconnectedError, PyOSError);

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

    // HTTP method strings
    /// The string "OPTIONS".
    pub options: Py<PyString>,
    /// The string "GET".
    pub get: Py<PyString>,
    /// The string "POST".
    pub post: Py<PyString>,
    /// The string "PUT".
    pub put: Py<PyString>,
    /// The string "DELETE".
    pub delete: Py<PyString>,
    /// The string "HEAD".
    pub head: Py<PyString>,
    /// The string "TRACE".
    pub trace: Py<PyString>,
    /// The string "PATCH".
    pub patch: Py<PyString>,

    // HTTP header name constants
    pub asgi_header_names: ASGIHeaderNameConstants,
    pub wsgi_header_names: WSGIHeaderNameConstants,

    // ASGI scope keys
    /// The string "asgi".
    pub asgi: Py<PyString>,
    /// The string "extensions".
    pub extensions: Py<PyString>,
    /// The string "type".
    pub typ: Py<PyString>,
    /// The string "lifespan".
    pub lifespan: Py<PyString>,
    /// The string "lifespan.startup"
    pub lifespan_startup: Py<PyString>,
    /// The string "lifespan.shutdown"
    pub lifespan_shutdown: Py<PyString>,
    /// The string "message".
    pub message: Py<PyString>,
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
    /// The string "state".
    pub state: Py<PyString>,
    /// The string "tls".
    pub tls: Py<PyString>,
    /// The string "tls_version".
    pub tls_version: Py<PyString>,
    /// The string "client_cert_name".
    pub client_cert_name: Py<PyString>,

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
    /// The string "create_task".
    pub create_task: Py<PyString>,
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
    /// The string "stop".
    pub stop: Py<PyString>,

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
    /// The string "QUERY_STRING".
    pub wsgi_query_string: Py<PyString>,
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
    /// The string "wsgi.errors".
    pub wsgi_errors: Py<PyString>,
    /// The string "wsgi.multithread".
    pub wsgi_multithread: Py<PyString>,
    /// The string "wsgi.multiprocess".
    pub wsgi_multiprocess: Py<PyString>,
    /// The string "wsgi.run_once".
    pub wsgi_run_once: Py<PyString>,
    /// The string "HTTP/1.0".
    pub wsgi_http_10: Py<PyString>,
    /// The string "HTTP/1.1".
    pub wsgi_http_11: Py<PyString>,
    /// The string "HTTP/2".
    pub wsgi_http_2: Py<PyString>,
    /// The string "HTTP/3".
    pub wsgi_http_3: Py<PyString>,
    /// The string "wsgi.ext.tls.tls_version".
    pub wsgi_ext_tls_version: Py<PyString>,
    /// The string "wsgi.ext.tls.client_cert_name".
    pub wsgi_ext_tls_client_cert_name: Py<PyString>,
    /// The string "wsgi.ext.http.send_trailers"
    pub wsgi_ext_http_send_trailers: Py<PyString>,

    // Misc
    /// The string "close".
    pub close: Py<PyString>,
    /// The string "with_traceback".
    pub with_traceback: Py<PyString>,
    /// An empty bytes object.
    pub empty_bytes: Py<PyBytes>,
    /// An empty string object.
    pub empty_string: Py<PyString>,

    /// The root path value passed from configuration.
    pub root_path_value: Py<PyString>,

    /// A singleton ClientDisconnectedError exception instance.
    /// The traceback is not important since it is caused by the client,
    /// so we can share it.
    pub client_disconnected_err: Py<PyAny>,
}

impl Constants {
    pub fn new(py: Python<'_>, root_path: &str) -> Self {
        let client_disconnected_err = ClientDisconnectedError::new_err(())
            .into_py_any(py)
            .unwrap();
        client_disconnected_err
            .setattr(py, "__traceback__", PyNone::get(py))
            .unwrap();
        Self {
            asgi: PyString::new(py, "asgi").unbind(),
            extensions: PyString::new(py, "extensions").unbind(),
            http: PyString::new(py, "http").unbind(),
            https: PyString::new(py, "https").unbind(),
            lifespan: PyString::new(py, "lifespan").unbind(),
            lifespan_startup: PyString::new(py, "lifespan.startup").unbind(),
            lifespan_shutdown: PyString::new(py, "lifespan.shutdown").unbind(),
            message: PyString::new(py, "message").unbind(),
            method: PyString::new(py, "method").unbind(),
            path: PyString::new(py, "path").unbind(),
            scheme: PyString::new(py, "scheme").unbind(),
            state: PyString::new(py, "state").unbind(),
            typ: PyString::new(py, "type").unbind(),
            tls: PyString::new(py, "tls").unbind(),
            tls_version: PyString::new(py, "tls_version").unbind(),
            client_cert_name: PyString::new(py, "client_cert_name").unbind(),

            http_version: PyString::new(py, "http_version").unbind(),
            http_10: PyString::new(py, "1.0").unbind(),
            http_11: PyString::new(py, "1.1").unbind(),
            http_2: PyString::new(py, "2").unbind(),
            http_3: PyString::new(py, "3").unbind(),

            asgi_header_names: ASGIHeaderNameConstants::new(py),
            wsgi_header_names: WSGIHeaderNameConstants::new(py),

            options: PyString::new(py, "OPTIONS").unbind(),
            get: PyString::new(py, "GET").unbind(),
            post: PyString::new(py, "POST").unbind(),
            put: PyString::new(py, "PUT").unbind(),
            delete: PyString::new(py, "DELETE").unbind(),
            head: PyString::new(py, "HEAD").unbind(),
            trace: PyString::new(py, "TRACE").unbind(),
            patch: PyString::new(py, "PATCH").unbind(),

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
            create_task: PyString::new(py, "create_task").unbind(),
            add_done_callback: PyString::new(py, "add_done_callback").unbind(),
            call_soon_threadsafe: PyString::new(py, "call_soon_threadsafe").unbind(),
            set_result: PyString::new(py, "set_result").unbind(),
            set_exception: PyString::new(py, "set_exception").unbind(),
            create_future: PyString::new(py, "create_future").unbind(),
            result: PyString::new(py, "result").unbind(),
            stop: PyString::new(py, "stop").unbind(),

            status: PyString::new(py, "status").unbind(),
            trailers: PyString::new(py, "trailers").unbind(),
            more_trailers: PyString::new(py, "more_trailers").unbind(),

            request_method: PyString::new(py, "REQUEST_METHOD").unbind(),
            script_name: PyString::new(py, "SCRIPT_NAME").unbind(),
            path_info: PyString::new(py, "PATH_INFO").unbind(),
            content_type: PyString::new(py, "CONTENT_TYPE").unbind(),
            content_length: PyString::new(py, "CONTENT_LENGTH").unbind(),
            wsgi_query_string: PyString::new(py, "QUERY_STRING").unbind(),
            server_name: PyString::new(py, "SERVER_NAME").unbind(),
            server_port: PyString::new(py, "SERVER_PORT").unbind(),
            server_protocol: PyString::new(py, "SERVER_PROTOCOL").unbind(),
            wsgi_version: PyString::new(py, "wsgi.version").unbind(),
            wsgi_url_scheme: PyString::new(py, "wsgi.url_scheme").unbind(),
            wsgi_input: PyString::new(py, "wsgi.input").unbind(),
            wsgi_errors: PyString::new(py, "wsgi.errors").unbind(),
            wsgi_multithread: PyString::new(py, "wsgi.multithread").unbind(),
            wsgi_multiprocess: PyString::new(py, "wsgi.multiprocess").unbind(),
            wsgi_run_once: PyString::new(py, "wsgi.run_once").unbind(),
            wsgi_http_10: PyString::new(py, "HTTP/1.0").unbind(),
            wsgi_http_11: PyString::new(py, "HTTP/1.1").unbind(),
            wsgi_http_2: PyString::new(py, "HTTP/2").unbind(),
            wsgi_http_3: PyString::new(py, "HTTP/3").unbind(),
            wsgi_ext_tls_version: PyString::new(py, "wsgi.ext.tls.tls_version").unbind(),
            wsgi_ext_tls_client_cert_name: PyString::new(py, "wsgi.ext.tls.client_cert_name")
                .unbind(),
            wsgi_ext_http_send_trailers: PyString::new(py, "wsgi.ext.http.send_trailers").unbind(),

            close: PyString::new(py, "close").unbind(),
            with_traceback: PyString::new(py, "with_traceback").unbind(),
            empty_bytes: PyBytes::new(py, b"").unbind(),
            empty_string: PyString::new(py, "").unbind(),

            root_path_value: PyString::new(py, root_path).unbind(),

            client_disconnected_err,
        }
    }
}

pub(crate) trait PyDictExt {
    fn set_http_method(
        &self,
        constants: &Constants,
        key: &Py<PyString>,
        method: &http::Method,
    ) -> PyResult<()>;

    fn set_http_scheme(
        &self,
        constants: &Constants,
        key: &Py<PyString>,
        scheme: &uri::Scheme,
    ) -> PyResult<()>;

    fn set_http_version(&self, constants: &Constants, version: &http::Version) -> PyResult<()>;

    fn set_http_version_wsgi(&self, constants: &Constants, version: &http::Version)
    -> PyResult<()>;
}

impl PyDictExt for Bound<'_, PyDict> {
    fn set_http_method(
        &self,
        constants: &Constants,
        key: &Py<PyString>,
        method: &http::Method,
    ) -> PyResult<()> {
        match *method {
            http::Method::OPTIONS => {
                self.set_item(key, &constants.options)?;
            }
            http::Method::GET => {
                self.set_item(key, &constants.get)?;
            }
            http::Method::POST => {
                self.set_item(key, &constants.post)?;
            }
            http::Method::PUT => {
                self.set_item(key, &constants.put)?;
            }
            http::Method::DELETE => {
                self.set_item(key, &constants.delete)?;
            }
            http::Method::HEAD => {
                self.set_item(key, &constants.head)?;
            }
            http::Method::TRACE => {
                self.set_item(key, &constants.trace)?;
            }
            http::Method::PATCH => {
                self.set_item(key, &constants.patch)?;
            }
            // We don't fast-path CONNECT since it can't really be used with apps,
            // it's for tunneling / websockets.
            _ => {
                self.set_item(key, method.as_str())?;
            }
        }
        Ok(())
    }

    fn set_http_scheme(
        &self,
        constants: &Constants,
        key: &Py<PyString>,
        scheme: &uri::Scheme,
    ) -> PyResult<()> {
        if *scheme == uri::Scheme::HTTP {
            self.set_item(key, &constants.http)?;
        } else if *scheme == uri::Scheme::HTTPS {
            self.set_item(key, &constants.https)?;
        } else {
            self.set_item(key, scheme.as_str())?;
        }
        Ok(())
    }

    fn set_http_version(&self, constants: &Constants, version: &http::Version) -> PyResult<()> {
        self.set_item(
            &constants.http_version,
            match *version {
                http::Version::HTTP_10 => &constants.http_10,
                http::Version::HTTP_11 => &constants.http_11,
                http::Version::HTTP_2 => &constants.http_2,
                http::Version::HTTP_3 => &constants.http_3,
                _ => &constants.http_11,
            },
        )?;
        Ok(())
    }

    fn set_http_version_wsgi(
        &self,
        constants: &Constants,
        version: &http::Version,
    ) -> PyResult<()> {
        self.set_item(
            &constants.server_protocol,
            match *version {
                http::Version::HTTP_10 => &constants.wsgi_http_10,
                http::Version::HTTP_11 => &constants.wsgi_http_11,
                http::Version::HTTP_2 => &constants.wsgi_http_2,
                http::Version::HTTP_3 => &constants.wsgi_http_3,
                _ => &constants.wsgi_http_11,
            },
        )?;
        Ok(())
    }
}

pub(crate) struct TlsInfo {
    pub tls_version: usize,
    pub client_cert_name: Option<Box<str>>,
}

pub(crate) struct Scope {
    pub http_version: http::Version,
    pub method: Method,
    pub scheme: Scheme,
    /// The full raw path, percent-encoded with query string.
    pub raw_path: Box<[u8]>,
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub client: Option<(Box<str>, i64)>,
    pub server: Option<(Box<str>, i64)>,
    pub tls_info: Option<TlsInfo>,
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

    let HeadersInfo {
        headers,
        method,
        raw_path,
        scheme,
    } = read_request_headers(envoy_filter);

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

    let tls_info = match envoy_filter
        .get_attribute_string(envoy_dynamic_module_type_attribute_id::ConnectionTlsVersion)
    {
        Some(tls_version) => {
            let tls_version = match tls_version.as_slice() {
                b"TLSv1.0" => 0x0301,
                b"TLSv1.1" => 0x0302,
                b"TLSv1.2" => 0x0303,
                b"TLSv1.3" => 0x0304,
                _ => 0,
            };
            let client_cert_name = envoy_filter
                .get_attribute_string(
                    envoy_dynamic_module_type_attribute_id::ConnectionSubjectPeerCertificate,
                )
                .map(|s| Box::from(str::from_utf8(s.as_slice()).unwrap_or("")));
            Some(TlsInfo {
                tls_version,
                client_cert_name,
            })
        }
        None => None,
    };

    Scope {
        http_version,
        method,
        scheme,
        raw_path,
        headers,
        client,
        server,
        tls_info,
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

/// Wrapper to mark Receiver as Sync. PyO3 hackily uses Sync as a signal for whether
/// a type is safe to be used with the GIL detached, even though the thread doesn't change.
/// We know it is fine for our usage.
pub(crate) struct SyncReceiver<T>(Receiver<T>);

impl<T> SyncReceiver<T> {
    pub fn new(receiver: Receiver<T>) -> Self {
        Self(receiver)
    }

    pub fn take(self) -> Receiver<T> {
        self.0
    }
}

impl<T> std::ops::Deref for SyncReceiver<T> {
    type Target = Receiver<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

unsafe impl<T> Sync for SyncReceiver<T> {}
