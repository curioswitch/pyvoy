use envoy_proxy_dynamic_modules_rust_sdk::{
    EnvoyHttpFilter, EnvoyHttpFilterScheduler, EnvoyMutBuffer,
};
use http::{HeaderName, HeaderValue, uri};

pub(crate) struct HeadersInfo {
    pub(crate) headers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) method: http::Method,
    pub(crate) raw_path: Box<[u8]>,
    pub(crate) scheme: uri::Scheme,
}

/// Reads request headers from Envoy, skipping pseudo-headers.
/// As this is an implementation detail of the filters, we return a Vec instead of Box
/// to avoid reallocation if shrinking (which would always happen for HTTP/2).
pub(crate) fn read_request_headers<EHF: EnvoyHttpFilter>(envoy_filter: &EHF) -> HeadersInfo {
    let envoy_headers = envoy_filter.get_request_headers();
    let mut headers = Vec::with_capacity(envoy_headers.len());
    let mut method = http::Method::GET;
    let mut raw_path: Box<[u8]> = Box::default();
    let mut scheme = uri::Scheme::HTTP;

    // ASGI says host must be the start of the vector so handle it first.
    if let Some(authority) = envoy_filter.get_request_header_value(":authority") {
        headers.push((
            http::header::HOST,
            HeaderValue::from_bytes(authority.as_slice())
                // Should't happen in practice.
                .unwrap_or_else(|_| HeaderValue::from_static("unknown")),
        ));
    }

    for (name_bytes, value_bytes) in envoy_headers.iter() {
        let name_slice = name_bytes.as_slice();
        let value_slice = value_bytes.as_slice();

        match name_slice {
            b":method" => {
                if let Ok(m) = http::Method::from_bytes(value_slice) {
                    method = m;
                }
                continue;
            }
            b":scheme" => {
                if let Ok(s) = uri::Scheme::try_from(value_slice) {
                    scheme = s;
                }
                continue;
            }
            b":path" => {
                raw_path = Box::from(value_slice);
                continue;
            }
            b":authority" => {
                // Already handled above.
                continue;
            }
            b"x-forwarded-proto" => {
                // There seems to be no way of disabling Envoy's addition of this. We don't need it
                // since we're not propagating to an upstream - we keep the scheme in the scope
                // instead.
                continue;
            }
            _ => {}
        }

        match (
            HeaderName::from_bytes(name_slice),
            HeaderValue::from_bytes(value_bytes.as_slice()),
        ) {
            (Ok(name), Ok(value)) => headers.push((name, value)),
            _ => continue,
        }
    }
    HeadersInfo {
        headers,
        method,
        raw_path,
        scheme,
    }
}

/// Checks if there is any request body that can be read.
pub(crate) fn has_request_body<EHF: EnvoyHttpFilter>(envoy_filter: &mut EHF) -> bool {
    if has_nonempty_buffer(&envoy_filter.get_buffered_request_body()) {
        return true;
    }

    if has_nonempty_buffer(&envoy_filter.get_received_request_body()) {
        return true;
    }

    false
}

fn has_nonempty_buffer(body: &Option<Vec<EnvoyMutBuffer<'_>>>) -> bool {
    if let Some(buffers) = body {
        buffers.iter().any(|buffer| !buffer.as_slice().is_empty())
    } else {
        false
    }
}

/// Reads the entire readable request body from Envoy.
pub(crate) fn read_request_body<EHF: EnvoyHttpFilter>(envoy_filter: &mut EHF) -> Box<[u8]> {
    let buffered_len = envoy_filter
        .get_buffered_request_body()
        .unwrap_or_default()
        .len();
    let received_len = envoy_filter
        .get_received_request_body()
        .unwrap_or_default()
        .len();

    match (buffered_len, received_len) {
        (0, 0) => Box::default(),
        (1, 0) => {
            let body: Box<[u8]> =
                Box::from(envoy_filter.get_buffered_request_body().unwrap()[0].as_slice());
            envoy_filter.drain_buffered_request_body(body.len());
            body
        }
        (0, 1) => {
            let body: Box<[u8]> =
                Box::from(envoy_filter.get_received_request_body().unwrap()[0].as_slice());
            envoy_filter.drain_received_request_body(body.len());
            body
        }
        _ => {
            let body_len = buffers_len(&envoy_filter.get_buffered_request_body())
                + buffers_len(&envoy_filter.get_received_request_body());
            let mut body = Vec::with_capacity(body_len);
            let buffered_read =
                extend_from_buffers(&envoy_filter.get_buffered_request_body(), &mut body);
            if buffered_read > 0 {
                envoy_filter.drain_buffered_request_body(buffered_read);
            }
            let received_read =
                extend_from_buffers(&envoy_filter.get_received_request_body(), &mut body);
            if received_read > 0 {
                envoy_filter.drain_received_request_body(received_read);
            }
            body.into_boxed_slice()
        }
    }
}

pub(crate) fn buffers_len(buffers: &Option<Vec<EnvoyMutBuffer<'_>>>) -> usize {
    if let Some(buffers) = buffers {
        buffers.iter().map(|b| b.as_slice().len()).sum()
    } else {
        0
    }
}

pub(crate) fn extend_from_buffers(
    buffers: &Option<Vec<EnvoyMutBuffer<'_>>>,
    vec: &mut Vec<u8>,
) -> usize {
    let mut read = 0;
    if let Some(buffers) = buffers {
        for buffer in buffers.iter().map(|b| b.as_slice()) {
            vec.extend_from_slice(buffer);
            read += buffer.len();
        }
    }
    read
}

/// A Sync wrapper around EnvoyHttpFilterScheduler.
///
/// TODO: Remove after https://github.com/envoyproxy/envoy/commit/c561059a04f496eda1e664a8d45bf9b64deef100 is released.
pub(crate) struct SyncScheduler(Box<dyn EnvoyHttpFilterScheduler>);

impl SyncScheduler {
    pub fn new(scheduler: Box<dyn EnvoyHttpFilterScheduler>) -> Self {
        Self(scheduler)
    }
}

impl std::ops::Deref for SyncScheduler {
    type Target = Box<dyn EnvoyHttpFilterScheduler>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

unsafe impl Sync for SyncScheduler {}
