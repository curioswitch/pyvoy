use std::{
    io::ErrorKind,
    sync::{Arc, Mutex},
};

use bytes::{BufMut as _, BytesMut};
use envoy_proxy_dynamic_modules_rust_sdk::{EnvoyHttpFilter, EnvoyMutBuffer};

#[derive(Default)]
struct Inner {
    read_buffer: BytesMut,
    write_buffer: BytesMut,
    /// The bytes of the frame being read that the WebSocket hasn't read yet, or
    /// `None` at a frame boundary. Reads stop at the end of each frame so the
    /// WebSocket never buffers the next one, which lets us inspect it first.
    frame_remaining: Option<usize>,
    /// Whether the frame being read is a ping or pong.
    frame_is_ping_or_pong: bool,
}

/// The byte stream of a WebSocket connection, carried by the request body from
/// the client and the response body to it.
///
/// The [`tungstenite::WebSocket`] owns one handle and reads from and writes to
/// the buffers, while the filter keeps another to move bytes between the buffers
/// and Envoy without borrowing the WebSocket.
#[derive(Clone, Default)]
pub(super) struct EnvoyStream {
    inner: Arc<Mutex<Inner>>,
}

impl EnvoyStream {
    /// Reads up to one block of the request body into our read buffer, draining
    /// only what we take so any remainder stays buffered in Envoy. Leaving data
    /// in Envoy's buffer is what lets it apply request backpressure once the
    /// application stops consuming.
    pub(super) fn read_block(&self, envoy_filter: &mut impl EnvoyHttpFilter) -> usize {
        const READ_BLOCK_SIZE: usize = 256 * 1024;
        let mut inner = self.inner.lock().unwrap();
        let buffered = take_block(
            &envoy_filter.get_buffered_request_body(),
            READ_BLOCK_SIZE,
            &mut inner.read_buffer,
        );
        if buffered > 0 {
            envoy_filter.drain_buffered_request_body(buffered);
        }
        let received = take_block(
            &envoy_filter.get_received_request_body(),
            READ_BLOCK_SIZE - buffered,
            &mut inner.read_buffer,
        );
        if received > 0 {
            envoy_filter.drain_received_request_body(received);
        }
        buffered + received
    }

    /// Whether the next frame for the WebSocket to read is a ping or pong it can
    /// read without failing, or `None` until enough of it has been read from
    /// Envoy to tell.
    pub(super) fn next_frame_is_ping_or_pong(&self) -> Option<bool> {
        let inner = self.inner.lock().unwrap();
        if inner.frame_remaining.is_some() {
            return Some(inner.frame_is_ping_or_pong);
        }
        is_ping_or_pong(&inner.read_buffer)
    }

    /// Takes the bytes the WebSocket has written so far, to send as response data.
    pub(super) fn take_written(&self) -> Option<BytesMut> {
        let mut inner = self.inner.lock().unwrap();
        if inner.write_buffer.is_empty() {
            return None;
        }
        Some(inner.write_buffer.split())
    }
}

/// Copies up to `limit` bytes from the buffers into `out`, returning how many.
fn take_block(
    buffers: &Option<Vec<EnvoyMutBuffer<'_>>>,
    limit: usize,
    out: &mut BytesMut,
) -> usize {
    let Some(buffers) = buffers else {
        return 0;
    };
    let mut remaining = limit;
    for buf in buffers {
        if remaining == 0 {
            break;
        }
        let slice = buf.as_slice();
        let take = slice.len().min(remaining);
        out.put(&slice[..take]);
        remaining -= take;
    }
    limit - remaining
}

/// Whether the frame starting with `head` is a final, unfragmented ping or pong
/// with no reserved bits, masked as frames from clients must be, and with a
/// payload short enough for a control frame. `None` until there are enough bytes
/// to tell.
fn is_ping_or_pong(head: &[u8]) -> Option<bool> {
    let (&first, &second) = (head.first()?, head.get(1)?);
    Some(matches!(first, 0x89 | 0x8A) && second & 0x80 != 0 && second & 0x7F <= 125)
}

/// The length of the frame starting with `head`, or `None` until there are
/// enough bytes to tell.
fn frame_len(head: &[u8]) -> Option<usize> {
    let second = *head.get(1)?;
    let (extended_len, payload_len) = match second & 0x7F {
        126 => (
            2,
            u64::from(u16::from_be_bytes(head.get(2..4)?.try_into().unwrap())),
        ),
        127 => (8, u64::from_be_bytes(head.get(2..10)?.try_into().unwrap())),
        len => (0, u64::from(len)),
    };
    let mask_len = if second & 0x80 != 0 { 4 } else { 0 };
    // The WebSocket fails on lengths over its maximum message size.
    let payload_len = usize::try_from(payload_len).unwrap_or(usize::MAX);
    Some((2usize + extended_len + mask_len).saturating_add(payload_len))
}

impl std::io::Read for EnvoyStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut inner = self.inner.lock().unwrap();
        let remaining = match inner.frame_remaining {
            Some(remaining) => remaining,
            None => {
                let Some(len) = frame_len(&inner.read_buffer) else {
                    return Err(ErrorKind::WouldBlock.into());
                };
                inner.frame_is_ping_or_pong = is_ping_or_pong(&inner.read_buffer) == Some(true);
                len
            }
        };
        let n = remaining.min(inner.read_buffer.len()).min(buf.len());
        if n == 0 {
            return Err(ErrorKind::WouldBlock.into());
        }
        buf[..n].copy_from_slice(&inner.read_buffer.split_to(n));
        inner.frame_remaining = Some(remaining - n).filter(|remaining| *remaining > 0);
        Ok(n)
    }
}

impl std::io::Write for EnvoyStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.lock().unwrap().write_buffer.put(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    fn stream(bytes: &[u8]) -> EnvoyStream {
        let stream = EnvoyStream::default();
        stream.inner.lock().unwrap().read_buffer.put(bytes);
        stream
    }

    #[test]
    fn reads_stop_at_frame_boundaries() {
        // A masked text frame with a 3-byte payload, then a masked ping.
        let text = [0x81, 0x83, 0, 0, 0, 0, b'a', b'b', b'c'];
        let ping = [0x89, 0x80, 0, 0, 0, 0];
        let mut stream = stream(&[&text[..], &ping[..]].concat());
        let mut buf = [0; 64];
        assert_eq!(stream.next_frame_is_ping_or_pong(), Some(false));
        assert_eq!(stream.read(&mut buf).unwrap(), text.len());
        assert_eq!(stream.next_frame_is_ping_or_pong(), Some(true));
        assert_eq!(stream.read(&mut buf).unwrap(), ping.len());
        assert_eq!(
            stream.read(&mut buf).unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
    }

    #[test]
    fn waits_for_extended_length() {
        let mut stream = stream(&[0x82, 0xFE, 0x01]);
        let mut buf = [0; 64];
        assert_eq!(
            stream.read(&mut buf).unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
        stream
            .inner
            .lock()
            .unwrap()
            .read_buffer
            .put(&[0x00, 1, 2, 3, 4][..]);
        // 4 header bytes, 4 mask bytes and a 256-byte payload, of which only the
        // mask has arrived.
        assert_eq!(stream.read(&mut buf).unwrap(), 8);
        assert_eq!(stream.inner.lock().unwrap().frame_remaining, Some(256));
        assert_eq!(stream.next_frame_is_ping_or_pong(), Some(false));
    }

    #[test]
    fn rejects_malformed_pings() {
        // Not final, unmasked, and too long for a control frame.
        assert_eq!(is_ping_or_pong(&[0x09, 0x80]), Some(false));
        assert_eq!(is_ping_or_pong(&[0x89, 0x00]), Some(false));
        assert_eq!(is_ping_or_pong(&[0x8A, 0xFE]), Some(false));
        assert_eq!(is_ping_or_pong(&[0x8A]), None);
    }
}
