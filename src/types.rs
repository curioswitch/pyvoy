use crate::envoy::*;
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilter;
use envoy_proxy_dynamic_modules_rust_sdk::abi::envoy_dynamic_module_type_attribute_id;
use http::{
    HeaderName, HeaderValue, Method, header,
    uri::{self, Scheme},
};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyString},
};

/// HTTP header name constants as PyBytes.
pub(crate) struct HeaderNameConstants {
    // HTTP header strings
    /// The string "accept"
    pub accept: Py<PyBytes>,
    /// The string "accept-charset"
    pub accept_charset: Py<PyBytes>,
    /// The string "accept-encoding"
    pub accept_encoding: Py<PyBytes>,
    /// The string "accept-language"
    pub accept_language: Py<PyBytes>,
    /// The string "accept-ranges"
    pub accept_ranges: Py<PyBytes>,
    /// The string "access-control-allow-credentials"
    pub access_control_allow_credentials: Py<PyBytes>,
    /// The string "access-control-allow-headers"
    pub access_control_allow_headers: Py<PyBytes>,
    /// The string "access-control-allow-methods"
    pub access_control_allow_methods: Py<PyBytes>,
    /// The string "access-control-allow-origin"
    pub access_control_allow_origin: Py<PyBytes>,
    /// The string "access-control-expose-headers"
    pub access_control_expose_headers: Py<PyBytes>,
    /// The string "access-control-max-age"
    pub access_control_max_age: Py<PyBytes>,
    /// The string "access-control-request-headers"
    pub access_control_request_headers: Py<PyBytes>,
    /// The string "access-control-request-method"
    pub access_control_request_method: Py<PyBytes>,
    /// The string "age"
    pub age: Py<PyBytes>,
    /// The string "allow"
    pub allow: Py<PyBytes>,
    /// The string "alt-svc"
    pub alt_svc: Py<PyBytes>,
    /// The string "authorization"
    pub authorization: Py<PyBytes>,
    /// The string "cache-control"
    pub cache_control: Py<PyBytes>,
    /// The string "cache-status"
    pub cache_status: Py<PyBytes>,
    /// The string "cdn-cache-control"
    pub cdn_cache_control: Py<PyBytes>,
    /// The string "connection"
    pub connection: Py<PyBytes>,
    /// The string "content-disposition"
    pub content_disposition: Py<PyBytes>,
    /// The string "content-encoding"
    pub content_encoding: Py<PyBytes>,
    /// The string "content-language"
    pub content_language: Py<PyBytes>,
    /// The string "content-length"
    pub content_length: Py<PyBytes>,
    /// The string "content-location"
    pub content_location: Py<PyBytes>,
    /// The string "content-range"
    pub content_range: Py<PyBytes>,
    /// The string "content-security-policy"
    pub content_security_policy: Py<PyBytes>,
    /// The string "content-security-policy-report-only"
    pub content_security_policy_report_only: Py<PyBytes>,
    /// The string "content-type"
    pub content_type: Py<PyBytes>,
    /// The string "cookie"
    pub cookie: Py<PyBytes>,
    /// The string "dnt"
    pub dnt: Py<PyBytes>,
    /// The string "date"
    pub date: Py<PyBytes>,
    /// The string "etag"
    pub etag: Py<PyBytes>,
    /// The string "expect"
    pub expect: Py<PyBytes>,
    /// The string "expires"
    pub expires: Py<PyBytes>,
    /// The string "forwarded"
    pub forwarded: Py<PyBytes>,
    /// The string "from"
    pub from: Py<PyBytes>,
    /// The string "host"
    pub host: Py<PyBytes>,
    /// The string "if-match"
    pub if_match: Py<PyBytes>,
    /// The string "if-modified-since"
    pub if_modified_since: Py<PyBytes>,
    /// The string "if-none-match"
    pub if_none_match: Py<PyBytes>,
    /// The string "if-range"
    pub if_range: Py<PyBytes>,
    /// The string "if-unmodified-since"
    pub if_unmodified_since: Py<PyBytes>,
    /// The string "last-modified"
    pub last_modified: Py<PyBytes>,
    /// The string "link"
    pub link: Py<PyBytes>,
    /// The string "location"
    pub location: Py<PyBytes>,
    /// The string "max-forwards"
    pub max_forwards: Py<PyBytes>,
    /// The string "origin"
    pub origin: Py<PyBytes>,
    /// The string "pragma"
    pub pragma: Py<PyBytes>,
    /// The string "proxy-authenticate"
    pub proxy_authenticate: Py<PyBytes>,
    /// The string "proxy-authorization"
    pub proxy_authorization: Py<PyBytes>,
    /// The string "public-key-pins"
    pub public_key_pins: Py<PyBytes>,
    /// The string "public-key-pins-report-only"
    pub public_key_pins_report_only: Py<PyBytes>,
    /// The string "range"
    pub range: Py<PyBytes>,
    /// The string "referer"
    pub referer: Py<PyBytes>,
    /// The string "referrer-policy"
    pub referrer_policy: Py<PyBytes>,
    /// The string "refresh"
    pub refresh: Py<PyBytes>,
    /// The string "retry-after"
    pub retry_after: Py<PyBytes>,
    /// The string "sec-websocket-accept"
    pub sec_websocket_accept: Py<PyBytes>,
    /// The string "sec-websocket-extensions"
    pub sec_websocket_extensions: Py<PyBytes>,
    /// The string "sec-websocket-key"
    pub sec_websocket_key: Py<PyBytes>,
    /// The string "sec-websocket-protocol"
    pub sec_websocket_protocol: Py<PyBytes>,
    /// The string "sec-websocket-version"
    pub sec_websocket_version: Py<PyBytes>,
    /// The string "server"
    pub server: Py<PyBytes>,
    /// The string "set-cookie"
    pub set_cookie: Py<PyBytes>,
    /// The string "strict-transport-security"
    pub strict_transport_security: Py<PyBytes>,
    /// The string "te"
    pub te: Py<PyBytes>,
    /// The string "trailer"
    pub trailer: Py<PyBytes>,
    /// The string "transfer-encoding"
    pub transfer_encoding: Py<PyBytes>,
    /// The string "user-agent"
    pub user_agent: Py<PyBytes>,
    /// The string "upgrade"
    pub upgrade: Py<PyBytes>,
    /// The string "upgrade-insecure-requests"
    pub upgrade_insecure_requests: Py<PyBytes>,
    /// The string "vary"
    pub vary: Py<PyBytes>,
    /// The string "via"
    pub via: Py<PyBytes>,
    /// The string "warning"
    pub warning: Py<PyBytes>,
    /// The string "www-authenticate"
    pub www_authenticate: Py<PyBytes>,
    /// The string "x-content-type-options"
    pub x_content_type_options: Py<PyBytes>,
    /// The string "x-dns-prefetch-control"
    pub x_dns_prefetch_control: Py<PyBytes>,
    /// The string "x-frame-options"
    pub x_frame_options: Py<PyBytes>,
    /// The string "x-xss-protection"
    pub x_xss_protection: Py<PyBytes>,
}

unsafe impl Sync for HeaderNameConstants {}

impl HeaderNameConstants {
    pub fn new(py: Python<'_>) -> Self {
        Self {
            accept: PyBytes::new(py, b"accept").unbind(),
            accept_charset: PyBytes::new(py, b"accept-charset").unbind(),
            accept_encoding: PyBytes::new(py, b"accept-encoding").unbind(),
            accept_language: PyBytes::new(py, b"accept-language").unbind(),
            accept_ranges: PyBytes::new(py, b"accept-ranges").unbind(),
            access_control_allow_credentials: PyBytes::new(py, b"access-control-allow-credentials")
                .unbind(),
            access_control_allow_headers: PyBytes::new(py, b"access-control-allow-headers")
                .unbind(),
            access_control_allow_methods: PyBytes::new(py, b"access-control-allow-methods")
                .unbind(),
            access_control_allow_origin: PyBytes::new(py, b"access-control-allow-origin").unbind(),
            access_control_expose_headers: PyBytes::new(py, b"access-control-expose-headers")
                .unbind(),
            access_control_max_age: PyBytes::new(py, b"access-control-max-age").unbind(),
            access_control_request_headers: PyBytes::new(py, b"access-control-request-headers")
                .unbind(),
            access_control_request_method: PyBytes::new(py, b"access-control-request-method")
                .unbind(),
            age: PyBytes::new(py, b"age").unbind(),
            allow: PyBytes::new(py, b"allow").unbind(),
            alt_svc: PyBytes::new(py, b"alt-svc").unbind(),
            authorization: PyBytes::new(py, b"authorization").unbind(),
            cache_control: PyBytes::new(py, b"cache-control").unbind(),
            cache_status: PyBytes::new(py, b"cache-status").unbind(),
            cdn_cache_control: PyBytes::new(py, b"cdn-cache-control").unbind(),
            connection: PyBytes::new(py, b"connection").unbind(),
            content_disposition: PyBytes::new(py, b"content-disposition").unbind(),
            content_encoding: PyBytes::new(py, b"content-encoding").unbind(),
            content_language: PyBytes::new(py, b"content-language").unbind(),
            content_length: PyBytes::new(py, b"content-length").unbind(),
            content_location: PyBytes::new(py, b"content-location").unbind(),
            content_range: PyBytes::new(py, b"content-range").unbind(),
            content_security_policy: PyBytes::new(py, b"content-security-policy").unbind(),
            content_security_policy_report_only: PyBytes::new(
                py,
                b"content-security-policy-report-only",
            )
            .unbind(),
            content_type: PyBytes::new(py, b"content-type").unbind(),
            cookie: PyBytes::new(py, b"cookie").unbind(),
            dnt: PyBytes::new(py, b"dnt").unbind(),
            date: PyBytes::new(py, b"date").unbind(),
            etag: PyBytes::new(py, b"etag").unbind(),
            expect: PyBytes::new(py, b"expect").unbind(),
            expires: PyBytes::new(py, b"expires").unbind(),
            forwarded: PyBytes::new(py, b"forwarded").unbind(),
            from: PyBytes::new(py, b"from").unbind(),
            host: PyBytes::new(py, b"host").unbind(),
            if_match: PyBytes::new(py, b"if-match").unbind(),
            if_modified_since: PyBytes::new(py, b"if-modified-since").unbind(),
            if_none_match: PyBytes::new(py, b"if-none-match").unbind(),
            if_range: PyBytes::new(py, b"if-range").unbind(),
            if_unmodified_since: PyBytes::new(py, b"if-unmodified-since").unbind(),
            last_modified: PyBytes::new(py, b"last-modified").unbind(),
            link: PyBytes::new(py, b"link").unbind(),
            location: PyBytes::new(py, b"location").unbind(),
            max_forwards: PyBytes::new(py, b"max-forwards").unbind(),
            origin: PyBytes::new(py, b"origin").unbind(),
            pragma: PyBytes::new(py, b"pragma").unbind(),
            proxy_authenticate: PyBytes::new(py, b"proxy-authenticate").unbind(),
            proxy_authorization: PyBytes::new(py, b"proxy-authorization").unbind(),
            public_key_pins: PyBytes::new(py, b"public-key-pins").unbind(),
            public_key_pins_report_only: PyBytes::new(py, b"public-key-pins-report-only").unbind(),
            range: PyBytes::new(py, b"range").unbind(),
            referer: PyBytes::new(py, b"referer").unbind(),
            referrer_policy: PyBytes::new(py, b"referrer-policy").unbind(),
            refresh: PyBytes::new(py, b"refresh").unbind(),
            retry_after: PyBytes::new(py, b"retry-after").unbind(),
            sec_websocket_accept: PyBytes::new(py, b"sec-websocket-accept").unbind(),
            sec_websocket_extensions: PyBytes::new(py, b"sec-websocket-extensions").unbind(),
            sec_websocket_key: PyBytes::new(py, b"sec-websocket-key").unbind(),
            sec_websocket_protocol: PyBytes::new(py, b"sec-websocket-protocol").unbind(),
            sec_websocket_version: PyBytes::new(py, b"sec-websocket-version").unbind(),
            server: PyBytes::new(py, b"server").unbind(),
            set_cookie: PyBytes::new(py, b"set-cookie").unbind(),
            strict_transport_security: PyBytes::new(py, b"strict-transport-security").unbind(),
            te: PyBytes::new(py, b"te").unbind(),
            trailer: PyBytes::new(py, b"trailer").unbind(),
            transfer_encoding: PyBytes::new(py, b"transfer-encoding").unbind(),
            user_agent: PyBytes::new(py, b"user-agent").unbind(),
            upgrade: PyBytes::new(py, b"upgrade").unbind(),
            upgrade_insecure_requests: PyBytes::new(py, b"upgrade-insecure-requests").unbind(),
            vary: PyBytes::new(py, b"vary").unbind(),
            via: PyBytes::new(py, b"via").unbind(),
            warning: PyBytes::new(py, b"warning").unbind(),
            www_authenticate: PyBytes::new(py, b"www-authenticate").unbind(),
            x_content_type_options: PyBytes::new(py, b"x-content-type-options").unbind(),
            x_dns_prefetch_control: PyBytes::new(py, b"x-dns-prefetch-control").unbind(),
            x_frame_options: PyBytes::new(py, b"x-frame-options").unbind(),
            x_xss_protection: PyBytes::new(py, b"x-xss-protection").unbind(),
        }
    }
}

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
    pub header_names: HeaderNameConstants,

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

    // Misc
    /// The string "close".
    pub close: Py<PyString>,
    pub empty_bytes: Py<PyBytes>,
    pub empty_string: Py<PyString>,
}

impl Constants {
    pub fn new(py: Python<'_>) -> Self {
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

            http_version: PyString::new(py, "http_version").unbind(),
            http_10: PyString::new(py, "1.0").unbind(),
            http_11: PyString::new(py, "1.1").unbind(),
            http_2: PyString::new(py, "2").unbind(),
            http_3: PyString::new(py, "3").unbind(),

            header_names: HeaderNameConstants::new(py),

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
            wsgi_multithread: PyString::new(py, "wsgi.multithread").unbind(),
            wsgi_multiprocess: PyString::new(py, "wsgi.multiprocess").unbind(),
            wsgi_run_once: PyString::new(py, "wsgi.run_once").unbind(),
            wsgi_http_10: PyString::new(py, "HTTP/1.0").unbind(),
            wsgi_http_11: PyString::new(py, "HTTP/1.1").unbind(),
            wsgi_http_2: PyString::new(py, "HTTP/2").unbind(),
            wsgi_http_3: PyString::new(py, "HTTP/3").unbind(),

            close: PyString::new(py, "close").unbind(),
            empty_bytes: PyBytes::new(py, b"").unbind(),
            empty_string: PyString::new(py, "").unbind(),
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

pub(crate) trait HeaderNameExt {
    fn as_py_bytes<'py>(&self, py: Python<'py>, constants: &Constants) -> Bound<'py, PyBytes>;
}

impl HeaderNameExt for HeaderName {
    fn as_py_bytes<'py>(&self, py: Python<'py>, constants: &Constants) -> Bound<'py, PyBytes> {
        match *self {
            header::ACCEPT => constants.header_names.accept.bind(py).to_owned(),
            header::ACCEPT_CHARSET => constants.header_names.accept_charset.bind(py).to_owned(),
            header::ACCEPT_ENCODING => constants.header_names.accept_encoding.bind(py).to_owned(),
            header::ACCEPT_LANGUAGE => constants.header_names.accept_language.bind(py).to_owned(),
            header::ACCEPT_RANGES => constants.header_names.accept_ranges.bind(py).to_owned(),
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS => constants
                .header_names
                .access_control_allow_credentials
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_HEADERS => constants
                .header_names
                .access_control_allow_headers
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_METHODS => constants
                .header_names
                .access_control_allow_methods
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_ORIGIN => constants
                .header_names
                .access_control_allow_origin
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_EXPOSE_HEADERS => constants
                .header_names
                .access_control_expose_headers
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_MAX_AGE => constants
                .header_names
                .access_control_max_age
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_REQUEST_HEADERS => constants
                .header_names
                .access_control_request_headers
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_REQUEST_METHOD => constants
                .header_names
                .access_control_request_method
                .bind(py)
                .to_owned(),
            header::AGE => constants.header_names.age.bind(py).to_owned(),
            header::ALLOW => constants.header_names.allow.bind(py).to_owned(),
            header::ALT_SVC => constants.header_names.alt_svc.bind(py).to_owned(),
            header::AUTHORIZATION => constants.header_names.authorization.bind(py).to_owned(),
            header::CACHE_CONTROL => constants.header_names.cache_control.bind(py).to_owned(),
            header::CACHE_STATUS => constants.header_names.cache_status.bind(py).to_owned(),
            header::CDN_CACHE_CONTROL => {
                constants.header_names.cdn_cache_control.bind(py).to_owned()
            }
            header::CONNECTION => constants.header_names.connection.bind(py).to_owned(),
            header::CONTENT_DISPOSITION => constants
                .header_names
                .content_disposition
                .bind(py)
                .to_owned(),
            header::CONTENT_ENCODING => constants.header_names.content_encoding.bind(py).to_owned(),
            header::CONTENT_LANGUAGE => constants.header_names.content_language.bind(py).to_owned(),
            header::CONTENT_LENGTH => constants.header_names.content_length.bind(py).to_owned(),
            header::CONTENT_LOCATION => constants.header_names.content_location.bind(py).to_owned(),
            header::CONTENT_RANGE => constants.header_names.content_range.bind(py).to_owned(),
            header::CONTENT_SECURITY_POLICY => constants
                .header_names
                .content_security_policy
                .bind(py)
                .to_owned(),
            header::CONTENT_SECURITY_POLICY_REPORT_ONLY => constants
                .header_names
                .content_security_policy_report_only
                .bind(py)
                .to_owned(),
            header::CONTENT_TYPE => constants.header_names.content_type.bind(py).to_owned(),
            header::COOKIE => constants.header_names.cookie.bind(py).to_owned(),
            header::DNT => constants.header_names.dnt.bind(py).to_owned(),
            header::DATE => constants.header_names.date.bind(py).to_owned(),
            header::ETAG => constants.header_names.etag.bind(py).to_owned(),
            header::EXPECT => constants.header_names.expect.bind(py).to_owned(),
            header::EXPIRES => constants.header_names.expires.bind(py).to_owned(),
            header::FORWARDED => constants.header_names.forwarded.bind(py).to_owned(),
            header::FROM => constants.header_names.from.bind(py).to_owned(),
            header::HOST => constants.header_names.host.bind(py).to_owned(),
            header::IF_MATCH => constants.header_names.if_match.bind(py).to_owned(),
            header::IF_MODIFIED_SINCE => {
                constants.header_names.if_modified_since.bind(py).to_owned()
            }
            header::IF_NONE_MATCH => constants.header_names.if_none_match.bind(py).to_owned(),
            header::IF_RANGE => constants.header_names.if_range.bind(py).to_owned(),
            header::IF_UNMODIFIED_SINCE => constants
                .header_names
                .if_unmodified_since
                .bind(py)
                .to_owned(),
            header::LAST_MODIFIED => constants.header_names.last_modified.bind(py).to_owned(),
            header::LINK => constants.header_names.link.bind(py).to_owned(),
            header::LOCATION => constants.header_names.location.bind(py).to_owned(),
            header::MAX_FORWARDS => constants.header_names.max_forwards.bind(py).to_owned(),
            header::ORIGIN => constants.header_names.origin.bind(py).to_owned(),
            header::PRAGMA => constants.header_names.pragma.bind(py).to_owned(),
            header::PROXY_AUTHENTICATE => constants
                .header_names
                .proxy_authenticate
                .bind(py)
                .to_owned(),
            header::PROXY_AUTHORIZATION => constants
                .header_names
                .proxy_authorization
                .bind(py)
                .to_owned(),
            header::PUBLIC_KEY_PINS => constants.header_names.public_key_pins.bind(py).to_owned(),
            header::PUBLIC_KEY_PINS_REPORT_ONLY => constants
                .header_names
                .public_key_pins_report_only
                .bind(py)
                .to_owned(),
            header::RANGE => constants.header_names.range.bind(py).to_owned(),
            header::REFERER => constants.header_names.referer.bind(py).to_owned(),
            header::REFERRER_POLICY => constants.header_names.referrer_policy.bind(py).to_owned(),
            header::REFRESH => constants.header_names.refresh.bind(py).to_owned(),
            header::RETRY_AFTER => constants.header_names.retry_after.bind(py).to_owned(),
            header::SEC_WEBSOCKET_ACCEPT => constants
                .header_names
                .sec_websocket_accept
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_EXTENSIONS => constants
                .header_names
                .sec_websocket_extensions
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_KEY => {
                constants.header_names.sec_websocket_key.bind(py).to_owned()
            }
            header::SEC_WEBSOCKET_PROTOCOL => constants
                .header_names
                .sec_websocket_protocol
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_VERSION => constants
                .header_names
                .sec_websocket_version
                .bind(py)
                .to_owned(),
            header::SERVER => constants.header_names.server.bind(py).to_owned(),
            header::SET_COOKIE => constants.header_names.set_cookie.bind(py).to_owned(),
            header::STRICT_TRANSPORT_SECURITY => constants
                .header_names
                .strict_transport_security
                .bind(py)
                .to_owned(),
            header::TE => constants.header_names.te.bind(py).to_owned(),
            header::TRAILER => constants.header_names.trailer.bind(py).to_owned(),
            header::TRANSFER_ENCODING => {
                constants.header_names.transfer_encoding.bind(py).to_owned()
            }
            header::USER_AGENT => constants.header_names.user_agent.bind(py).to_owned(),
            header::UPGRADE => constants.header_names.upgrade.bind(py).to_owned(),
            header::UPGRADE_INSECURE_REQUESTS => constants
                .header_names
                .upgrade_insecure_requests
                .bind(py)
                .to_owned(),
            header::VARY => constants.header_names.vary.bind(py).to_owned(),
            header::VIA => constants.header_names.via.bind(py).to_owned(),
            header::WARNING => constants.header_names.warning.bind(py).to_owned(),
            header::WWW_AUTHENTICATE => constants.header_names.www_authenticate.bind(py).to_owned(),
            header::X_CONTENT_TYPE_OPTIONS => constants
                .header_names
                .x_content_type_options
                .bind(py)
                .to_owned(),
            header::X_DNS_PREFETCH_CONTROL => constants
                .header_names
                .x_dns_prefetch_control
                .bind(py)
                .to_owned(),
            header::X_FRAME_OPTIONS => constants.header_names.x_frame_options.bind(py).to_owned(),
            header::X_XSS_PROTECTION => constants.header_names.x_xss_protection.bind(py).to_owned(),
            _ => PyBytes::new(py, self.as_str().as_bytes()),
        }
    }
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

    Scope {
        http_version,
        method,
        scheme,
        raw_path,
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
