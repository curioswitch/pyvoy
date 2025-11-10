use envoy_proxy_dynamic_modules_rust_sdk::{EnvoyHttpFilter, EnvoyHttpFilterScheduler};
use http::{HeaderName, HeaderValue};

/// Reads request headers from Envoy, skipping pseudo-headers.
/// As this is an implementation detail of the filters, we return a Vec instead of Box
/// to avoid reallocation if shrinking (which would always happen for HTTP/2).
pub(crate) fn read_request_headers<EHF: EnvoyHttpFilter>(
    envoy_filter: &EHF,
) -> Vec<(HeaderName, HeaderValue)> {
    let envoy_headers = envoy_filter.get_request_headers();
    let mut headers = Vec::with_capacity(envoy_headers.len());
    for (name_bytes, value_bytes) in envoy_headers.iter() {
        let name_slice = name_bytes.as_slice();

        match name_slice {
            b":method" | b":scheme" | b":authority" | b":path" => {
                // Pseudo-headers are skipped. We don't guard by http_version because Envoy will
                // normalize to always include them.
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
    headers
}

/// Checks if there is any request body that can be read.
pub(crate) fn has_request_body<EHF: EnvoyHttpFilter>(envoy_filter: &mut EHF) -> bool {
    envoy_filter
        .get_request_body()
        .map(|buffers| buffers.iter().any(|buffer| !buffer.as_slice().is_empty()))
        .unwrap_or(false)
}

/// Reads the entire readable request body from Envoy.
pub(crate) fn read_request_body<EHF: EnvoyHttpFilter>(envoy_filter: &mut EHF) -> Box<[u8]> {
    let buffers = envoy_filter.get_request_body().unwrap_or_default();
    match buffers.len() {
        0 => Box::default(),
        1 => {
            let body: Box<[u8]> = Box::from(buffers[0].as_slice());
            envoy_filter.drain_request_body(body.len());
            body
        }
        _ => {
            let body_len = buffers.iter().map(|b| b.as_slice().len()).sum();
            let mut body = Vec::with_capacity(body_len);
            for buffer in buffers {
                body.extend_from_slice(buffer.as_slice());
            }
            envoy_filter.drain_request_body(body.len());
            body.into_boxed_slice()
        }
    }
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
