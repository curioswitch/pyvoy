use http::{HeaderName, header};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyString},
};

use crate::types::Constants;

/// ASGI HTTP header name constants as lowercase PyBytes (e.g., "accept", "content-type").
pub(crate) struct ASGIHeaderNameConstants {
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
    /// The string "user-agent"
    pub user_agent: Py<PyBytes>,
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

unsafe impl Sync for ASGIHeaderNameConstants {}

impl ASGIHeaderNameConstants {
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
            user_agent: PyBytes::new(py, b"user-agent").unbind(),
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

/// WSGI HTTP header name constants as uppercase PyString (e.g., "HTTP_ACCEPT", "HTTP_CONTENT_TYPE").
pub(crate) struct WSGIHeaderNameConstants {
    /// The string "HTTP_ACCEPT"
    pub http_accept: Py<PyString>,
    /// The string "HTTP_ACCEPT_CHARSET"
    pub http_accept_charset: Py<PyString>,
    /// The string "HTTP_ACCEPT_ENCODING"
    pub http_accept_encoding: Py<PyString>,
    /// The string "HTTP_ACCEPT_LANGUAGE"
    pub http_accept_language: Py<PyString>,
    /// The string "HTTP_ACCEPT_RANGES"
    pub http_accept_ranges: Py<PyString>,
    /// The string "HTTP_ACCESS_CONTROL_ALLOW_CREDENTIALS"
    pub http_access_control_allow_credentials: Py<PyString>,
    /// The string "HTTP_ACCESS_CONTROL_ALLOW_HEADERS"
    pub http_access_control_allow_headers: Py<PyString>,
    /// The string "HTTP_ACCESS_CONTROL_ALLOW_METHODS"
    pub http_access_control_allow_methods: Py<PyString>,
    /// The string "HTTP_ACCESS_CONTROL_ALLOW_ORIGIN"
    pub http_access_control_allow_origin: Py<PyString>,
    /// The string "HTTP_ACCESS_CONTROL_EXPOSE_HEADERS"
    pub http_access_control_expose_headers: Py<PyString>,
    /// The string "HTTP_ACCESS_CONTROL_MAX_AGE"
    pub http_access_control_max_age: Py<PyString>,
    /// The string "HTTP_ACCESS_CONTROL_REQUEST_HEADERS"
    pub http_access_control_request_headers: Py<PyString>,
    /// The string "HTTP_ACCESS_CONTROL_REQUEST_METHOD"
    pub http_access_control_request_method: Py<PyString>,
    /// The string "HTTP_AGE"
    pub http_age: Py<PyString>,
    /// The string "HTTP_ALLOW"
    pub http_allow: Py<PyString>,
    /// The string "HTTP_ALT_SVC"
    pub http_alt_svc: Py<PyString>,
    /// The string "HTTP_AUTHORIZATION"
    pub http_authorization: Py<PyString>,
    /// The string "HTTP_CACHE_CONTROL"
    pub http_cache_control: Py<PyString>,
    /// The string "HTTP_CACHE_STATUS"
    pub http_cache_status: Py<PyString>,
    /// The string "HTTP_CDN_CACHE_CONTROL"
    pub http_cdn_cache_control: Py<PyString>,
    /// The string "HTTP_CONTENT_DISPOSITION"
    pub http_content_disposition: Py<PyString>,
    /// The string "HTTP_CONTENT_ENCODING"
    pub http_content_encoding: Py<PyString>,
    /// The string "HTTP_CONTENT_LANGUAGE"
    pub http_content_language: Py<PyString>,
    /// The string "HTTP_CONTENT_LOCATION"
    pub http_content_location: Py<PyString>,
    /// The string "HTTP_CONTENT_RANGE"
    pub http_content_range: Py<PyString>,
    /// The string "HTTP_CONTENT_SECURITY_POLICY"
    pub http_content_security_policy: Py<PyString>,
    /// The string "HTTP_CONTENT_SECURITY_POLICY_REPORT_ONLY"
    pub http_content_security_policy_report_only: Py<PyString>,
    /// The string "HTTP_COOKIE"
    pub http_cookie: Py<PyString>,
    /// The string "HTTP_DNT"
    pub http_dnt: Py<PyString>,
    /// The string "HTTP_DATE"
    pub http_date: Py<PyString>,
    /// The string "HTTP_ETAG"
    pub http_etag: Py<PyString>,
    /// The string "HTTP_EXPECT"
    pub http_expect: Py<PyString>,
    /// The string "HTTP_EXPIRES"
    pub http_expires: Py<PyString>,
    /// The string "HTTP_FORWARDED"
    pub http_forwarded: Py<PyString>,
    /// The string "HTTP_FROM"
    pub http_from: Py<PyString>,
    /// The string "HTTP_HOST"
    pub http_host: Py<PyString>,
    /// The string "HTTP_IF_MATCH"
    pub http_if_match: Py<PyString>,
    /// The string "HTTP_IF_MODIFIED_SINCE"
    pub http_if_modified_since: Py<PyString>,
    /// The string "HTTP_IF_NONE_MATCH"
    pub http_if_none_match: Py<PyString>,
    /// The string "HTTP_IF_RANGE"
    pub http_if_range: Py<PyString>,
    /// The string "HTTP_IF_UNMODIFIED_SINCE"
    pub http_if_unmodified_since: Py<PyString>,
    /// The string "HTTP_LAST_MODIFIED"
    pub http_last_modified: Py<PyString>,
    /// The string "HTTP_LINK"
    pub http_link: Py<PyString>,
    /// The string "HTTP_LOCATION"
    pub http_location: Py<PyString>,
    /// The string "HTTP_MAX_FORWARDS"
    pub http_max_forwards: Py<PyString>,
    /// The string "HTTP_ORIGIN"
    pub http_origin: Py<PyString>,
    /// The string "HTTP_PRAGMA"
    pub http_pragma: Py<PyString>,
    /// The string "HTTP_PROXY_AUTHENTICATE"
    pub http_proxy_authenticate: Py<PyString>,
    /// The string "HTTP_PROXY_AUTHORIZATION"
    pub http_proxy_authorization: Py<PyString>,
    /// The string "HTTP_PUBLIC_KEY_PINS"
    pub http_public_key_pins: Py<PyString>,
    /// The string "HTTP_PUBLIC_KEY_PINS_REPORT_ONLY"
    pub http_public_key_pins_report_only: Py<PyString>,
    /// The string "HTTP_RANGE"
    pub http_range: Py<PyString>,
    /// The string "HTTP_REFERER"
    pub http_referer: Py<PyString>,
    /// The string "HTTP_REFERRER_POLICY"
    pub http_referrer_policy: Py<PyString>,
    /// The string "HTTP_REFRESH"
    pub http_refresh: Py<PyString>,
    /// The string "HTTP_RETRY_AFTER"
    pub http_retry_after: Py<PyString>,
    /// The string "HTTP_SEC_WEBSOCKET_ACCEPT"
    pub http_sec_websocket_accept: Py<PyString>,
    /// The string "HTTP_SEC_WEBSOCKET_EXTENSIONS"
    pub http_sec_websocket_extensions: Py<PyString>,
    /// The string "HTTP_SEC_WEBSOCKET_KEY"
    pub http_sec_websocket_key: Py<PyString>,
    /// The string "HTTP_SEC_WEBSOCKET_PROTOCOL"
    pub http_sec_websocket_protocol: Py<PyString>,
    /// The string "HTTP_SEC_WEBSOCKET_VERSION"
    pub http_sec_websocket_version: Py<PyString>,
    /// The string "HTTP_SERVER"
    pub http_server: Py<PyString>,
    /// The string "HTTP_SET_COOKIE"
    pub http_set_cookie: Py<PyString>,
    /// The string "HTTP_STRICT_TRANSPORT_SECURITY"
    pub http_strict_transport_security: Py<PyString>,
    /// The string "HTTP_TE"
    pub http_te: Py<PyString>,
    /// The string "HTTP_TRAILER"
    pub http_trailer: Py<PyString>,
    /// The string "HTTP_USER_AGENT"
    pub http_user_agent: Py<PyString>,
    /// The string "HTTP_UPGRADE_INSECURE_REQUESTS"
    pub http_upgrade_insecure_requests: Py<PyString>,
    /// The string "HTTP_VARY"
    pub http_vary: Py<PyString>,
    /// The string "HTTP_VIA"
    pub http_via: Py<PyString>,
    /// The string "HTTP_WARNING"
    pub http_warning: Py<PyString>,
    /// The string "HTTP_WWW_AUTHENTICATE"
    pub http_www_authenticate: Py<PyString>,
    /// The string "HTTP_X_CONTENT_TYPE_OPTIONS"
    pub http_x_content_type_options: Py<PyString>,
    /// The string "HTTP_X_DNS_PREFETCH_CONTROL"
    pub http_x_dns_prefetch_control: Py<PyString>,
    /// The string "HTTP_X_FRAME_OPTIONS"
    pub http_x_frame_options: Py<PyString>,
    /// The string "HTTP_X_XSS_PROTECTION"
    pub http_x_xss_protection: Py<PyString>,
}

unsafe impl Sync for WSGIHeaderNameConstants {}

impl WSGIHeaderNameConstants {
    pub fn new(py: Python<'_>) -> Self {
        Self {
            http_accept: PyString::new(py, "HTTP_ACCEPT").unbind(),
            http_accept_charset: PyString::new(py, "HTTP_ACCEPT_CHARSET").unbind(),
            http_accept_encoding: PyString::new(py, "HTTP_ACCEPT_ENCODING").unbind(),
            http_accept_language: PyString::new(py, "HTTP_ACCEPT_LANGUAGE").unbind(),
            http_accept_ranges: PyString::new(py, "HTTP_ACCEPT_RANGES").unbind(),
            http_access_control_allow_credentials: PyString::new(
                py,
                "HTTP_ACCESS_CONTROL_ALLOW_CREDENTIALS",
            )
            .unbind(),
            http_access_control_allow_headers: PyString::new(
                py,
                "HTTP_ACCESS_CONTROL_ALLOW_HEADERS",
            )
            .unbind(),
            http_access_control_allow_methods: PyString::new(
                py,
                "HTTP_ACCESS_CONTROL_ALLOW_METHODS",
            )
            .unbind(),
            http_access_control_allow_origin: PyString::new(py, "HTTP_ACCESS_CONTROL_ALLOW_ORIGIN")
                .unbind(),
            http_access_control_expose_headers: PyString::new(
                py,
                "HTTP_ACCESS_CONTROL_EXPOSE_HEADERS",
            )
            .unbind(),
            http_access_control_max_age: PyString::new(py, "HTTP_ACCESS_CONTROL_MAX_AGE").unbind(),
            http_access_control_request_headers: PyString::new(
                py,
                "HTTP_ACCESS_CONTROL_REQUEST_HEADERS",
            )
            .unbind(),
            http_access_control_request_method: PyString::new(
                py,
                "HTTP_ACCESS_CONTROL_REQUEST_METHOD",
            )
            .unbind(),
            http_age: PyString::new(py, "HTTP_AGE").unbind(),
            http_allow: PyString::new(py, "HTTP_ALLOW").unbind(),
            http_alt_svc: PyString::new(py, "HTTP_ALT_SVC").unbind(),
            http_authorization: PyString::new(py, "HTTP_AUTHORIZATION").unbind(),
            http_cache_control: PyString::new(py, "HTTP_CACHE_CONTROL").unbind(),
            http_cache_status: PyString::new(py, "HTTP_CACHE_STATUS").unbind(),
            http_cdn_cache_control: PyString::new(py, "HTTP_CDN_CACHE_CONTROL").unbind(),
            http_content_disposition: PyString::new(py, "HTTP_CONTENT_DISPOSITION").unbind(),
            http_content_encoding: PyString::new(py, "HTTP_CONTENT_ENCODING").unbind(),
            http_content_language: PyString::new(py, "HTTP_CONTENT_LANGUAGE").unbind(),
            http_content_location: PyString::new(py, "HTTP_CONTENT_LOCATION").unbind(),
            http_content_range: PyString::new(py, "HTTP_CONTENT_RANGE").unbind(),
            http_content_security_policy: PyString::new(py, "HTTP_CONTENT_SECURITY_POLICY")
                .unbind(),
            http_content_security_policy_report_only: PyString::new(
                py,
                "HTTP_CONTENT_SECURITY_POLICY_REPORT_ONLY",
            )
            .unbind(),
            http_cookie: PyString::new(py, "HTTP_COOKIE").unbind(),
            http_dnt: PyString::new(py, "HTTP_DNT").unbind(),
            http_date: PyString::new(py, "HTTP_DATE").unbind(),
            http_etag: PyString::new(py, "HTTP_ETAG").unbind(),
            http_expect: PyString::new(py, "HTTP_EXPECT").unbind(),
            http_expires: PyString::new(py, "HTTP_EXPIRES").unbind(),
            http_forwarded: PyString::new(py, "HTTP_FORWARDED").unbind(),
            http_from: PyString::new(py, "HTTP_FROM").unbind(),
            http_host: PyString::new(py, "HTTP_HOST").unbind(),
            http_if_match: PyString::new(py, "HTTP_IF_MATCH").unbind(),
            http_if_modified_since: PyString::new(py, "HTTP_IF_MODIFIED_SINCE").unbind(),
            http_if_none_match: PyString::new(py, "HTTP_IF_NONE_MATCH").unbind(),
            http_if_range: PyString::new(py, "HTTP_IF_RANGE").unbind(),
            http_if_unmodified_since: PyString::new(py, "HTTP_IF_UNMODIFIED_SINCE").unbind(),
            http_last_modified: PyString::new(py, "HTTP_LAST_MODIFIED").unbind(),
            http_link: PyString::new(py, "HTTP_LINK").unbind(),
            http_location: PyString::new(py, "HTTP_LOCATION").unbind(),
            http_max_forwards: PyString::new(py, "HTTP_MAX_FORWARDS").unbind(),
            http_origin: PyString::new(py, "HTTP_ORIGIN").unbind(),
            http_pragma: PyString::new(py, "HTTP_PRAGMA").unbind(),
            http_proxy_authenticate: PyString::new(py, "HTTP_PROXY_AUTHENTICATE").unbind(),
            http_proxy_authorization: PyString::new(py, "HTTP_PROXY_AUTHORIZATION").unbind(),
            http_public_key_pins: PyString::new(py, "HTTP_PUBLIC_KEY_PINS").unbind(),
            http_public_key_pins_report_only: PyString::new(py, "HTTP_PUBLIC_KEY_PINS_REPORT_ONLY")
                .unbind(),
            http_range: PyString::new(py, "HTTP_RANGE").unbind(),
            http_referer: PyString::new(py, "HTTP_REFERER").unbind(),
            http_referrer_policy: PyString::new(py, "HTTP_REFERRER_POLICY").unbind(),
            http_refresh: PyString::new(py, "HTTP_REFRESH").unbind(),
            http_retry_after: PyString::new(py, "HTTP_RETRY_AFTER").unbind(),
            http_sec_websocket_accept: PyString::new(py, "HTTP_SEC_WEBSOCKET_ACCEPT").unbind(),
            http_sec_websocket_extensions: PyString::new(py, "HTTP_SEC_WEBSOCKET_EXTENSIONS")
                .unbind(),
            http_sec_websocket_key: PyString::new(py, "HTTP_SEC_WEBSOCKET_KEY").unbind(),
            http_sec_websocket_protocol: PyString::new(py, "HTTP_SEC_WEBSOCKET_PROTOCOL").unbind(),
            http_sec_websocket_version: PyString::new(py, "HTTP_SEC_WEBSOCKET_VERSION").unbind(),
            http_server: PyString::new(py, "HTTP_SERVER").unbind(),
            http_set_cookie: PyString::new(py, "HTTP_SET_COOKIE").unbind(),
            http_strict_transport_security: PyString::new(py, "HTTP_STRICT_TRANSPORT_SECURITY")
                .unbind(),
            http_te: PyString::new(py, "HTTP_TE").unbind(),
            http_trailer: PyString::new(py, "HTTP_TRAILER").unbind(),
            http_user_agent: PyString::new(py, "HTTP_USER_AGENT").unbind(),
            http_upgrade_insecure_requests: PyString::new(py, "HTTP_UPGRADE_INSECURE_REQUESTS")
                .unbind(),
            http_vary: PyString::new(py, "HTTP_VARY").unbind(),
            http_via: PyString::new(py, "HTTP_VIA").unbind(),
            http_warning: PyString::new(py, "HTTP_WARNING").unbind(),
            http_www_authenticate: PyString::new(py, "HTTP_WWW_AUTHENTICATE").unbind(),
            http_x_content_type_options: PyString::new(py, "HTTP_X_CONTENT_TYPE_OPTIONS").unbind(),
            http_x_dns_prefetch_control: PyString::new(py, "HTTP_X_DNS_PREFETCH_CONTROL").unbind(),
            http_x_frame_options: PyString::new(py, "HTTP_X_FRAME_OPTIONS").unbind(),
            http_x_xss_protection: PyString::new(py, "HTTP_X_XSS_PROTECTION").unbind(),
        }
    }
}

pub(crate) trait HeaderNameExt {
    fn to_asgi_bytes<'py>(&self, py: Python<'py>, constants: &Constants) -> Bound<'py, PyBytes>;
    fn to_wsgi_string<'py>(&self, py: Python<'py>, constants: &Constants) -> Bound<'py, PyString>;
}

impl HeaderNameExt for HeaderName {
    fn to_asgi_bytes<'py>(&self, py: Python<'py>, constants: &Constants) -> Bound<'py, PyBytes> {
        match *self {
            header::ACCEPT => constants.asgi_header_names.accept.bind(py).to_owned(),
            header::ACCEPT_CHARSET => constants
                .asgi_header_names
                .accept_charset
                .bind(py)
                .to_owned(),
            header::ACCEPT_ENCODING => constants
                .asgi_header_names
                .accept_encoding
                .bind(py)
                .to_owned(),
            header::ACCEPT_LANGUAGE => constants
                .asgi_header_names
                .accept_language
                .bind(py)
                .to_owned(),
            header::ACCEPT_RANGES => constants
                .asgi_header_names
                .accept_ranges
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS => constants
                .asgi_header_names
                .access_control_allow_credentials
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_HEADERS => constants
                .asgi_header_names
                .access_control_allow_headers
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_METHODS => constants
                .asgi_header_names
                .access_control_allow_methods
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_ORIGIN => constants
                .asgi_header_names
                .access_control_allow_origin
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_EXPOSE_HEADERS => constants
                .asgi_header_names
                .access_control_expose_headers
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_MAX_AGE => constants
                .asgi_header_names
                .access_control_max_age
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_REQUEST_HEADERS => constants
                .asgi_header_names
                .access_control_request_headers
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_REQUEST_METHOD => constants
                .asgi_header_names
                .access_control_request_method
                .bind(py)
                .to_owned(),
            header::AGE => constants.asgi_header_names.age.bind(py).to_owned(),
            header::ALLOW => constants.asgi_header_names.allow.bind(py).to_owned(),
            header::ALT_SVC => constants.asgi_header_names.alt_svc.bind(py).to_owned(),
            header::AUTHORIZATION => constants
                .asgi_header_names
                .authorization
                .bind(py)
                .to_owned(),
            header::CACHE_CONTROL => constants
                .asgi_header_names
                .cache_control
                .bind(py)
                .to_owned(),
            header::CACHE_STATUS => constants.asgi_header_names.cache_status.bind(py).to_owned(),
            header::CDN_CACHE_CONTROL => constants
                .asgi_header_names
                .cdn_cache_control
                .bind(py)
                .to_owned(),
            header::CONTENT_DISPOSITION => constants
                .asgi_header_names
                .content_disposition
                .bind(py)
                .to_owned(),
            header::CONTENT_ENCODING => constants
                .asgi_header_names
                .content_encoding
                .bind(py)
                .to_owned(),
            header::CONTENT_LANGUAGE => constants
                .asgi_header_names
                .content_language
                .bind(py)
                .to_owned(),
            header::CONTENT_LENGTH => constants
                .asgi_header_names
                .content_length
                .bind(py)
                .to_owned(),
            header::CONTENT_LOCATION => constants
                .asgi_header_names
                .content_location
                .bind(py)
                .to_owned(),
            header::CONTENT_RANGE => constants
                .asgi_header_names
                .content_range
                .bind(py)
                .to_owned(),
            header::CONTENT_SECURITY_POLICY => constants
                .asgi_header_names
                .content_security_policy
                .bind(py)
                .to_owned(),
            header::CONTENT_SECURITY_POLICY_REPORT_ONLY => constants
                .asgi_header_names
                .content_security_policy_report_only
                .bind(py)
                .to_owned(),
            header::CONTENT_TYPE => constants.asgi_header_names.content_type.bind(py).to_owned(),
            header::COOKIE => constants.asgi_header_names.cookie.bind(py).to_owned(),
            header::DNT => constants.asgi_header_names.dnt.bind(py).to_owned(),
            header::DATE => constants.asgi_header_names.date.bind(py).to_owned(),
            header::ETAG => constants.asgi_header_names.etag.bind(py).to_owned(),
            header::EXPECT => constants.asgi_header_names.expect.bind(py).to_owned(),
            header::EXPIRES => constants.asgi_header_names.expires.bind(py).to_owned(),
            header::FORWARDED => constants.asgi_header_names.forwarded.bind(py).to_owned(),
            header::FROM => constants.asgi_header_names.from.bind(py).to_owned(),
            header::HOST => constants.asgi_header_names.host.bind(py).to_owned(),
            header::IF_MATCH => constants.asgi_header_names.if_match.bind(py).to_owned(),
            header::IF_MODIFIED_SINCE => constants
                .asgi_header_names
                .if_modified_since
                .bind(py)
                .to_owned(),
            header::IF_NONE_MATCH => constants
                .asgi_header_names
                .if_none_match
                .bind(py)
                .to_owned(),
            header::IF_RANGE => constants.asgi_header_names.if_range.bind(py).to_owned(),
            header::IF_UNMODIFIED_SINCE => constants
                .asgi_header_names
                .if_unmodified_since
                .bind(py)
                .to_owned(),
            header::LAST_MODIFIED => constants
                .asgi_header_names
                .last_modified
                .bind(py)
                .to_owned(),
            header::LINK => constants.asgi_header_names.link.bind(py).to_owned(),
            header::LOCATION => constants.asgi_header_names.location.bind(py).to_owned(),
            header::MAX_FORWARDS => constants.asgi_header_names.max_forwards.bind(py).to_owned(),
            header::ORIGIN => constants.asgi_header_names.origin.bind(py).to_owned(),
            header::PRAGMA => constants.asgi_header_names.pragma.bind(py).to_owned(),
            header::PROXY_AUTHENTICATE => constants
                .asgi_header_names
                .proxy_authenticate
                .bind(py)
                .to_owned(),
            header::PROXY_AUTHORIZATION => constants
                .asgi_header_names
                .proxy_authorization
                .bind(py)
                .to_owned(),
            header::PUBLIC_KEY_PINS => constants
                .asgi_header_names
                .public_key_pins
                .bind(py)
                .to_owned(),
            header::PUBLIC_KEY_PINS_REPORT_ONLY => constants
                .asgi_header_names
                .public_key_pins_report_only
                .bind(py)
                .to_owned(),
            header::RANGE => constants.asgi_header_names.range.bind(py).to_owned(),
            header::REFERER => constants.asgi_header_names.referer.bind(py).to_owned(),
            header::REFERRER_POLICY => constants
                .asgi_header_names
                .referrer_policy
                .bind(py)
                .to_owned(),
            header::REFRESH => constants.asgi_header_names.refresh.bind(py).to_owned(),
            header::RETRY_AFTER => constants.asgi_header_names.retry_after.bind(py).to_owned(),
            header::SEC_WEBSOCKET_ACCEPT => constants
                .asgi_header_names
                .sec_websocket_accept
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_EXTENSIONS => constants
                .asgi_header_names
                .sec_websocket_extensions
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_KEY => constants
                .asgi_header_names
                .sec_websocket_key
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_PROTOCOL => constants
                .asgi_header_names
                .sec_websocket_protocol
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_VERSION => constants
                .asgi_header_names
                .sec_websocket_version
                .bind(py)
                .to_owned(),
            header::SERVER => constants.asgi_header_names.server.bind(py).to_owned(),
            header::SET_COOKIE => constants.asgi_header_names.set_cookie.bind(py).to_owned(),
            header::STRICT_TRANSPORT_SECURITY => constants
                .asgi_header_names
                .strict_transport_security
                .bind(py)
                .to_owned(),
            header::TE => constants.asgi_header_names.te.bind(py).to_owned(),
            header::TRAILER => constants.asgi_header_names.trailer.bind(py).to_owned(),
            header::USER_AGENT => constants.asgi_header_names.user_agent.bind(py).to_owned(),
            header::UPGRADE_INSECURE_REQUESTS => constants
                .asgi_header_names
                .upgrade_insecure_requests
                .bind(py)
                .to_owned(),
            header::VARY => constants.asgi_header_names.vary.bind(py).to_owned(),
            header::VIA => constants.asgi_header_names.via.bind(py).to_owned(),
            header::WARNING => constants.asgi_header_names.warning.bind(py).to_owned(),
            header::WWW_AUTHENTICATE => constants
                .asgi_header_names
                .www_authenticate
                .bind(py)
                .to_owned(),
            header::X_CONTENT_TYPE_OPTIONS => constants
                .asgi_header_names
                .x_content_type_options
                .bind(py)
                .to_owned(),
            header::X_DNS_PREFETCH_CONTROL => constants
                .asgi_header_names
                .x_dns_prefetch_control
                .bind(py)
                .to_owned(),
            header::X_FRAME_OPTIONS => constants
                .asgi_header_names
                .x_frame_options
                .bind(py)
                .to_owned(),
            header::X_XSS_PROTECTION => constants
                .asgi_header_names
                .x_xss_protection
                .bind(py)
                .to_owned(),
            _ => PyBytes::new(py, self.as_str().as_bytes()),
        }
    }

    fn to_wsgi_string<'py>(&self, py: Python<'py>, constants: &Constants) -> Bound<'py, PyString> {
        match *self {
            header::ACCEPT => constants.wsgi_header_names.http_accept.bind(py).to_owned(),
            header::ACCEPT_CHARSET => constants
                .wsgi_header_names
                .http_accept_charset
                .bind(py)
                .to_owned(),
            header::ACCEPT_ENCODING => constants
                .wsgi_header_names
                .http_accept_encoding
                .bind(py)
                .to_owned(),
            header::ACCEPT_LANGUAGE => constants
                .wsgi_header_names
                .http_accept_language
                .bind(py)
                .to_owned(),
            header::ACCEPT_RANGES => constants
                .wsgi_header_names
                .http_accept_ranges
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS => constants
                .wsgi_header_names
                .http_access_control_allow_credentials
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_HEADERS => constants
                .wsgi_header_names
                .http_access_control_allow_headers
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_METHODS => constants
                .wsgi_header_names
                .http_access_control_allow_methods
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_ALLOW_ORIGIN => constants
                .wsgi_header_names
                .http_access_control_allow_origin
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_EXPOSE_HEADERS => constants
                .wsgi_header_names
                .http_access_control_expose_headers
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_MAX_AGE => constants
                .wsgi_header_names
                .http_access_control_max_age
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_REQUEST_HEADERS => constants
                .wsgi_header_names
                .http_access_control_request_headers
                .bind(py)
                .to_owned(),
            header::ACCESS_CONTROL_REQUEST_METHOD => constants
                .wsgi_header_names
                .http_access_control_request_method
                .bind(py)
                .to_owned(),
            header::AGE => constants.wsgi_header_names.http_age.bind(py).to_owned(),
            header::ALLOW => constants.wsgi_header_names.http_allow.bind(py).to_owned(),
            header::ALT_SVC => constants.wsgi_header_names.http_alt_svc.bind(py).to_owned(),
            header::AUTHORIZATION => constants
                .wsgi_header_names
                .http_authorization
                .bind(py)
                .to_owned(),
            header::CACHE_CONTROL => constants
                .wsgi_header_names
                .http_cache_control
                .bind(py)
                .to_owned(),
            header::CACHE_STATUS => constants
                .wsgi_header_names
                .http_cache_status
                .bind(py)
                .to_owned(),
            header::CDN_CACHE_CONTROL => constants
                .wsgi_header_names
                .http_cdn_cache_control
                .bind(py)
                .to_owned(),
            header::CONTENT_DISPOSITION => constants
                .wsgi_header_names
                .http_content_disposition
                .bind(py)
                .to_owned(),
            header::CONTENT_ENCODING => constants
                .wsgi_header_names
                .http_content_encoding
                .bind(py)
                .to_owned(),
            header::CONTENT_LANGUAGE => constants
                .wsgi_header_names
                .http_content_language
                .bind(py)
                .to_owned(),
            header::CONTENT_LOCATION => constants
                .wsgi_header_names
                .http_content_location
                .bind(py)
                .to_owned(),
            header::CONTENT_RANGE => constants
                .wsgi_header_names
                .http_content_range
                .bind(py)
                .to_owned(),
            header::CONTENT_SECURITY_POLICY => constants
                .wsgi_header_names
                .http_content_security_policy
                .bind(py)
                .to_owned(),
            header::CONTENT_SECURITY_POLICY_REPORT_ONLY => constants
                .wsgi_header_names
                .http_content_security_policy_report_only
                .bind(py)
                .to_owned(),
            header::COOKIE => constants.wsgi_header_names.http_cookie.bind(py).to_owned(),
            header::DNT => constants.wsgi_header_names.http_dnt.bind(py).to_owned(),
            header::DATE => constants.wsgi_header_names.http_date.bind(py).to_owned(),
            header::ETAG => constants.wsgi_header_names.http_etag.bind(py).to_owned(),
            header::EXPECT => constants.wsgi_header_names.http_expect.bind(py).to_owned(),
            header::EXPIRES => constants.wsgi_header_names.http_expires.bind(py).to_owned(),
            header::FORWARDED => constants
                .wsgi_header_names
                .http_forwarded
                .bind(py)
                .to_owned(),
            header::FROM => constants.wsgi_header_names.http_from.bind(py).to_owned(),
            header::HOST => constants.wsgi_header_names.http_host.bind(py).to_owned(),
            header::IF_MATCH => constants
                .wsgi_header_names
                .http_if_match
                .bind(py)
                .to_owned(),
            header::IF_MODIFIED_SINCE => constants
                .wsgi_header_names
                .http_if_modified_since
                .bind(py)
                .to_owned(),
            header::IF_NONE_MATCH => constants
                .wsgi_header_names
                .http_if_none_match
                .bind(py)
                .to_owned(),
            header::IF_RANGE => constants
                .wsgi_header_names
                .http_if_range
                .bind(py)
                .to_owned(),
            header::IF_UNMODIFIED_SINCE => constants
                .wsgi_header_names
                .http_if_unmodified_since
                .bind(py)
                .to_owned(),
            header::LAST_MODIFIED => constants
                .wsgi_header_names
                .http_last_modified
                .bind(py)
                .to_owned(),
            header::LINK => constants.wsgi_header_names.http_link.bind(py).to_owned(),
            header::LOCATION => constants
                .wsgi_header_names
                .http_location
                .bind(py)
                .to_owned(),
            header::MAX_FORWARDS => constants
                .wsgi_header_names
                .http_max_forwards
                .bind(py)
                .to_owned(),
            header::ORIGIN => constants.wsgi_header_names.http_origin.bind(py).to_owned(),
            header::PRAGMA => constants.wsgi_header_names.http_pragma.bind(py).to_owned(),
            header::PROXY_AUTHENTICATE => constants
                .wsgi_header_names
                .http_proxy_authenticate
                .bind(py)
                .to_owned(),
            header::PROXY_AUTHORIZATION => constants
                .wsgi_header_names
                .http_proxy_authorization
                .bind(py)
                .to_owned(),
            header::PUBLIC_KEY_PINS => constants
                .wsgi_header_names
                .http_public_key_pins
                .bind(py)
                .to_owned(),
            header::PUBLIC_KEY_PINS_REPORT_ONLY => constants
                .wsgi_header_names
                .http_public_key_pins_report_only
                .bind(py)
                .to_owned(),
            header::RANGE => constants.wsgi_header_names.http_range.bind(py).to_owned(),
            header::REFERER => constants.wsgi_header_names.http_referer.bind(py).to_owned(),
            header::REFERRER_POLICY => constants
                .wsgi_header_names
                .http_referrer_policy
                .bind(py)
                .to_owned(),
            header::REFRESH => constants.wsgi_header_names.http_refresh.bind(py).to_owned(),
            header::RETRY_AFTER => constants
                .wsgi_header_names
                .http_retry_after
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_ACCEPT => constants
                .wsgi_header_names
                .http_sec_websocket_accept
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_EXTENSIONS => constants
                .wsgi_header_names
                .http_sec_websocket_extensions
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_KEY => constants
                .wsgi_header_names
                .http_sec_websocket_key
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_PROTOCOL => constants
                .wsgi_header_names
                .http_sec_websocket_protocol
                .bind(py)
                .to_owned(),
            header::SEC_WEBSOCKET_VERSION => constants
                .wsgi_header_names
                .http_sec_websocket_version
                .bind(py)
                .to_owned(),
            header::SERVER => constants.wsgi_header_names.http_server.bind(py).to_owned(),
            header::SET_COOKIE => constants
                .wsgi_header_names
                .http_set_cookie
                .bind(py)
                .to_owned(),
            header::STRICT_TRANSPORT_SECURITY => constants
                .wsgi_header_names
                .http_strict_transport_security
                .bind(py)
                .to_owned(),
            header::TE => constants.wsgi_header_names.http_te.bind(py).to_owned(),
            header::TRAILER => constants.wsgi_header_names.http_trailer.bind(py).to_owned(),
            header::USER_AGENT => constants
                .wsgi_header_names
                .http_user_agent
                .bind(py)
                .to_owned(),
            header::UPGRADE_INSECURE_REQUESTS => constants
                .wsgi_header_names
                .http_upgrade_insecure_requests
                .bind(py)
                .to_owned(),
            header::VARY => constants.wsgi_header_names.http_vary.bind(py).to_owned(),
            header::VIA => constants.wsgi_header_names.http_via.bind(py).to_owned(),
            header::WARNING => constants.wsgi_header_names.http_warning.bind(py).to_owned(),
            header::WWW_AUTHENTICATE => constants
                .wsgi_header_names
                .http_www_authenticate
                .bind(py)
                .to_owned(),
            header::X_CONTENT_TYPE_OPTIONS => constants
                .wsgi_header_names
                .http_x_content_type_options
                .bind(py)
                .to_owned(),
            header::X_DNS_PREFETCH_CONTROL => constants
                .wsgi_header_names
                .http_x_dns_prefetch_control
                .bind(py)
                .to_owned(),
            header::X_FRAME_OPTIONS => constants
                .wsgi_header_names
                .http_x_frame_options
                .bind(py)
                .to_owned(),
            header::X_XSS_PROTECTION => constants
                .wsgi_header_names
                .http_x_xss_protection
                .bind(py)
                .to_owned(),
            _ => {
                // For custom headers, convert to WSGI format: uppercase with underscores and HTTP_ prefix
                let header_str = self.as_str().to_uppercase().replace('-', "_");
                PyString::new(py, &format!("HTTP_{}", header_str))
            }
        }
    }
}
