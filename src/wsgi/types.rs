use http::{HeaderName, HeaderValue, StatusCode};

/// The event ID to trigger the filter to process request events.
pub(crate) const EVENT_ID_REQUEST: u64 = 1;

/// The event ID to trigger the filter to process response events.
pub(crate) const EVENT_ID_RESPONSE: u64 = 2;

/// An event to read request the body, called from the WSGI input stream,
/// read by the filter when scheduling [`EVENT_ID_REQUEST`].
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

/// An event to begin a response with headers.
pub(crate) struct ResponseStartEvent {
    /// The status code of the response.
    pub status: StatusCode,
    /// The headers of the response.
    pub headers: Vec<(HeaderName, HeaderValue)>,
}

/// An event to send a chunk of the response body.
pub(crate) struct ResponseBodyEvent {
    /// The body bytes to send.
    pub body: Box<[u8]>,
    /// Whether there is more body to send after this chunk.
    pub more_body: bool,
}

/// An event to send a response, read by the filter when scheduling [`EVENT_ID_RESPONSE`].
pub(crate) enum ResponseEvent {
    /// Start the response with headers and the first body chunk.
    Start(ResponseStartEvent, ResponseBodyEvent),
    /// Send a chunk of the response body.
    Body(ResponseBodyEvent),
    /// Send response trailers.
    Trailers(Option<ResponseStartEvent>, Vec<(HeaderName, HeaderValue)>),
    /// Complete the response with failure due to an application exception.
    Exception,
}
