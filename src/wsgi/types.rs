use http::{HeaderName, HeaderValue};

pub(crate) struct ResponseStartEvent {
    pub status: u16,
    pub headers: Vec<(HeaderName, HeaderValue)>,
}

pub(crate) struct ResponseBodyEvent {
    pub body: Box<[u8]>,
    pub more_body: bool,
}

pub(crate) enum ResponseEvent {
    Start(ResponseStartEvent, ResponseBodyEvent),
    Body(ResponseBodyEvent),
    Exception,
}

/// An event to read request the body, called from the WSGI input stream.
///
/// size 0 is handled in the WSGI input stream itself so is never set here.
///
/// https://peps.python.org/pep-3333/#input-and-error-streams
pub(crate) enum RequestReadEvent {
    /// Wait until a request to read.
    Wait,

    /// A raw read of up to the given number of bytes. If the bytes is negative, read until EOF.
    /// https://docs.python.org/3.14/library/io.html#io.RawIOBase.read
    Raw(isize),
    /// A read of a line of up to the given number of bytes. If the bytes is negative, read until
    /// the first line or EOF.
    /// https://docs.python.org/3.14/library/io.html#io.IOBase.readline
    Line(isize),
}

/// A chunk read from the request body. If `closed` is true, this is the final chunk.
pub(crate) struct RequestBody {
    /// The body bytes read.
    pub body: Box<[u8]>,
    /// Whether the request body is closed, meaning this is the last chunk.
    pub closed: bool,
}

pub(crate) const EVENT_ID_REQUEST: u64 = 1;
pub(crate) const EVENT_ID_RESPONSE: u64 = 2;
pub(crate) const EVENT_ID_EXCEPTION: u64 = 3;
