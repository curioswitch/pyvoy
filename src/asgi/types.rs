use pyo3::prelude::{Py, PyAny};

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
    Custom(Box<[u8]>),
}

pub(crate) enum HttpScheme {
    Http,
    Https,
}

pub(crate) struct Scope {
    pub http_version: HttpVersion,
    pub method: HttpMethod,
    pub scheme: HttpScheme,
    pub raw_path: Box<[u8]>,
    pub query_string: Box<[u8]>,
    pub headers: Vec<(Box<[u8]>, Box<[u8]>)>,
    pub client: Option<(String, i64)>,
    pub server: Option<(String, i64)>,
}

pub(crate) struct ResponseStartEvent {
    pub headers: Vec<(String, Box<[u8]>)>,
    pub trailers: bool,
}

pub(crate) struct ResponseBodyEvent {
    pub body: Box<[u8]>,
    pub more_body: bool,
    pub future: Py<PyAny>,
}

pub(crate) struct ResponseTrailersEvent {
    pub headers: Vec<(String, Box<[u8]>)>,
    pub more_trailers: bool,
}

pub(crate) enum ResponseEvent {
    Start(ResponseStartEvent, ResponseBodyEvent),
    Body(ResponseBodyEvent),
    Trailers(ResponseTrailersEvent),
    Exception,
}

pub(crate) const EVENT_ID_REQUEST: u64 = 1;
pub(crate) const EVENT_ID_RESPONSE: u64 = 2;
pub(crate) const EVENT_ID_EXCEPTION: u64 = 3;
