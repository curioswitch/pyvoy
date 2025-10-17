pub(crate) struct ResponseStartEvent {
    pub headers: Vec<(String, Box<[u8]>)>,
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

pub(crate) struct RequestBody {
    pub body: Box<[u8]>,
    pub closed: bool,
}

pub(crate) const EVENT_ID_REQUEST: u64 = 1;
pub(crate) const EVENT_ID_RESPONSE: u64 = 2;
pub(crate) const EVENT_ID_EXCEPTION: u64 = 3;
