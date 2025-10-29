use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilter;
use envoy_proxy_dynamic_modules_rust_sdk::abi::envoy_dynamic_module_type_attribute_id;
use pyo3::{
    intern,
    prelude::*,
    types::{PyDict, PyString},
};

pub(crate) enum HttpVersion {
    Http10,
    Http11,
    Http2,
    Http3,
}

pub(crate) enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Delete,
    Connect,
    Options,
    Trace,
    Patch,
    Custom(Box<str>),
}

impl HttpMethod {
    pub(crate) fn set_in_dict<'py>(
        &self,
        py: Python<'py>,
        dict: &Bound<'py, PyDict>,
        key: &Bound<'py, PyString>,
    ) -> PyResult<()> {
        match self {
            HttpMethod::Get => dict.set_item(key, intern!(py, "GET"))?,
            HttpMethod::Head => dict.set_item(key, intern!(py, "HEAD"))?,
            HttpMethod::Post => dict.set_item(key, intern!(py, "POST"))?,
            HttpMethod::Put => dict.set_item(key, intern!(py, "PUT"))?,
            HttpMethod::Delete => dict.set_item(key, intern!(py, "DELETE"))?,
            HttpMethod::Connect => dict.set_item(key, intern!(py, "CONNECT"))?,
            HttpMethod::Options => dict.set_item(key, intern!(py, "OPTIONS"))?,
            HttpMethod::Trace => dict.set_item(key, intern!(py, "TRACE"))?,
            HttpMethod::Patch => dict.set_item(key, intern!(py, "PATCH"))?,
            HttpMethod::Custom(m) => {
                dict.set_item(key, PyString::new(py, m))?;
            }
        };
        Ok(())
    }
}

pub(crate) enum HttpScheme {
    Http,
    Https,
}

fn is_pseudoheader(http_version: &HttpVersion, name: &[u8]) -> bool {
    matches!(http_version, HttpVersion::Http2 | HttpVersion::Http3)
        && matches!(name, b":method" | b":scheme" | b":authority" | b":path")
}

pub(crate) struct Scope {
    pub http_version: HttpVersion,
    pub method: HttpMethod,
    pub scheme: HttpScheme,
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
            b"HTTP/1.0" => HttpVersion::Http10,
            b"HTTP/1.1" => HttpVersion::Http11,
            b"HTTP/2" => HttpVersion::Http2,
            b"HTTP/3" => HttpVersion::Http3,
            _ => HttpVersion::Http11,
        },
        None => HttpVersion::Http11,
    };
    let method = match envoy_filter
        .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestMethod)
    {
        Some(v) => match v.as_slice() {
            b"GET" => HttpMethod::Get,
            b"HEAD" => HttpMethod::Head,
            b"POST" => HttpMethod::Post,
            b"PUT" => HttpMethod::Put,
            b"DELETE" => HttpMethod::Delete,
            b"CONNECT" => HttpMethod::Connect,
            b"OPTIONS" => HttpMethod::Options,
            b"TRACE" => HttpMethod::Trace,
            b"PATCH" => HttpMethod::Patch,
            other => HttpMethod::Custom(Box::from(str::from_utf8(other).unwrap_or(""))),
        },
        None => HttpMethod::Get,
    };

    let scheme = match envoy_filter
        .get_attribute_string(envoy_dynamic_module_type_attribute_id::RequestScheme)
    {
        Some(v) => match v.as_slice() {
            b"http" => HttpScheme::Http,
            b"https" => HttpScheme::Https,
            _ => HttpScheme::Http,
        },
        None => HttpScheme::Http,
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
