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

impl std::io::Read for EnvoyStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut inner = self.inner.lock().unwrap();
        if inner.read_buffer.is_empty() {
            return Err(ErrorKind::WouldBlock.into());
        }
        let n = inner.read_buffer.len().min(buf.len());
        buf[..n].copy_from_slice(&inner.read_buffer.split_to(n));
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
