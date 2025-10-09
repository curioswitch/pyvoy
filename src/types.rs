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
