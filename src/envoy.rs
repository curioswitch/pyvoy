use envoy_proxy_dynamic_modules_rust_sdk::{EnvoyHttpFilter, EnvoyHttpFilterScheduler};
use http::{HeaderName, HeaderValue, uri};
#[cfg(Py_GIL_DISABLED)]
use pyo3::Py;
use pyo3::{
    Bound, Python,
    types::{PyBytes, PyBytesMethods as _},
};

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
    envoy_filter
        .get_request_body()
        .map(|buffers| buffers.iter().any(|buffer| !buffer.as_slice().is_empty()))
        .unwrap_or(false)
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

/// A slice of bytes. On free-threaded Python, we can use PyBytes directly
/// within the filter too because there is no need to take the GIL to read
/// the bytes.
pub(crate) enum ByteSlice {
    /// Bytes safe to read from Envoy request threads on any Python.
    Gil(Box<[u8]>),

    #[cfg(Py_GIL_DISABLED)]
    /// Bytes only safe to read from Envoy request threads in free-threaded Python.
    NoGil(Py<PyBytes>),
}

impl ByteSlice {
    /// Reads available Envoy request data into a [`Byteslice`].
    pub(crate) fn from_envoy<EHF: EnvoyHttpFilter>(envoy_filter: &mut EHF) -> ByteSlice {
        let buffers = envoy_filter.get_request_body().unwrap_or_default();
        match buffers.len() {
            0 => ByteSlice::Gil(Box::default()),
            1 => {
                let buffer = buffers[0].as_slice();
                let len = buffer.len();

                #[cfg(Py_GIL_DISABLED)]
                let body = if len == 0 {
                    ByteSlice::Gil(Box::default())
                } else {
                    ByteSlice::NoGil(Python::attach(|py| PyBytes::new(py, buffer).unbind()))
                };

                #[cfg(not(Py_GIL_DISABLED))]
                let body = ByteSlice::Gil(Box::from(buffer));

                envoy_filter.drain_request_body(len);
                body
            }
            _ => {
                // We can't avoid a copy if we need to concatenate buffers since Python bytes is immutable.
                // So we just stick to the Box version for this branch.
                let body_len = buffers.iter().map(|b| b.as_slice().len()).sum();
                let mut body = Vec::with_capacity(body_len);
                for buffer in buffers {
                    body.extend_from_slice(buffer.as_slice());
                }
                envoy_filter.drain_request_body(body.len());
                ByteSlice::Gil(body.into_boxed_slice())
            }
        }
    }

    /// Creates a new [`ByteSlice`] from a Python bytes object.
    pub(crate) fn from_py<'py>(bytes: &Bound<'py, PyBytes>) -> ByteSlice {
        if bytes.as_bytes().is_empty() {
            return ByteSlice::Gil(Box::default());
        }

        #[cfg(not(Py_GIL_DISABLED))]
        return ByteSlice::Gil(Box::from(bytes.as_bytes()));

        #[cfg(Py_GIL_DISABLED)]
        return ByteSlice::NoGil(bytes.clone().unbind());
    }

    /// Returns whether this [`ByteSlice`] is empty.
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            ByteSlice::Gil(bytes) => bytes.is_empty(),
            #[cfg(Py_GIL_DISABLED)]
            // We always use Gil for empty bytes in new().
            ByteSlice::NoGil(_) => false,
        }
    }

    /// Converts this [`ByteSlice`] to a [`PyBytes`].
    pub(crate) fn to_py<'py>(self, py: Python<'py>) -> Bound<'py, PyBytes> {
        match self {
            ByteSlice::Gil(bytes) => PyBytes::new(py, &bytes),
            #[cfg(Py_GIL_DISABLED)]
            ByteSlice::NoGil(bytes) => bytes.into_bound(py),
        }
    }

    /// Sends the bytes in this [`ByteSlice`] in the Envoy response.
    pub(crate) fn send_response_data<EHF: EnvoyHttpFilter>(
        self,
        envoy_filter: &mut EHF,
        end_stream: bool,
    ) {
        match self {
            ByteSlice::Gil(bytes) => {
                envoy_filter.send_response_data(&bytes, end_stream);
            }
            #[cfg(Py_GIL_DISABLED)]
            ByteSlice::NoGil(bytes) => Python::attach(|py| {
                envoy_filter.send_response_data(bytes.as_bytes(py), end_stream);
            }),
        }
    }
}
