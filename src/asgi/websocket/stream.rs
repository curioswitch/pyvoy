use std::{
    io::ErrorKind,
    sync::{Arc, Mutex},
};

use bytes::{BufMut as _, BytesMut};
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyNetworkFilter;

struct Inner {
    read_buffer: BytesMut,
    total_read: usize,
    write_buffer: BytesMut,
    closed: bool,
}

#[derive(Clone)]
pub(super) struct EnvoyStream {
    inner: Arc<Mutex<Inner>>,
}

impl EnvoyStream {
    /// Reads data from the Envoy buffer during handshake. We do not drain the buffer
    /// to be able to send it to the filter chain if the handshake fails.
    pub(super) fn read_from_buffered(&self, envoy_filter: &mut impl EnvoyNetworkFilter) {
        let mut inner = self.inner.lock().unwrap();
        let (buffers, total_size) = envoy_filter.get_read_buffer_chunks();
        let to_read = total_size - inner.total_read;
        inner.read_buffer.reserve(to_read);
        let mut to_skip = inner.total_read;
        for buf in buffers {
            if to_skip == 0 {
                inner.read_buffer.put(buf.as_slice());
                continue;
            }
            let buf_slice = buf.as_slice();
            if to_skip >= buf_slice.len() {
                to_skip -= buf_slice.len();
                continue;
            }
            let start = to_skip;
            let end = buf_slice.len();
            inner.read_buffer.put(&buf_slice[start..end]);
            to_skip = 0;
        }
        inner.total_read += to_read;
    }

    /// Reads data from the Envoy buffer after handshake. We don't keep any bytes buffered anymore
    /// so always drain each read.
    pub(super) fn read_from(&self, envoy_filter: &mut impl EnvoyNetworkFilter) {
        let mut inner = self.inner.lock().unwrap();
        let (buffers, total_size) = envoy_filter.get_read_buffer_chunks();
        inner.read_buffer.reserve(total_size);
        for buf in buffers {
            inner.read_buffer.put(buf.as_slice());
        }
        inner.total_read += total_size;
        envoy_filter.drain_read_buffer(total_size);
    }

    pub(super) fn put(&self, data: &[u8]) {
        let mut inner = self.inner.lock().unwrap();
        inner.read_buffer.put(data);
        inner.total_read += data.len();
    }

    pub(super) fn write_to(&self, envoy_filter: &mut impl EnvoyNetworkFilter) {
        let mut inner = self.inner.lock().unwrap();
        if inner.write_buffer.is_empty() {
            return;
        }
        envoy_filter.write(&inner.write_buffer, false);
        inner.write_buffer.clear();
    }
}

impl Default for EnvoyStream {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                read_buffer: BytesMut::new(),
                total_read: 0,
                write_buffer: BytesMut::new(),
                closed: false,
            })),
        }
    }
}

impl std::io::Read for EnvoyStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut inner = self.inner.lock().unwrap();
        if inner.read_buffer.is_empty() {
            return if inner.closed {
                Ok(0)
            } else {
                Err(ErrorKind::WouldBlock.into())
            };
        }
        let n = std::cmp::min(buf.len(), inner.read_buffer.len());
        let split = inner.read_buffer.split_to(n);
        buf[..n].copy_from_slice(&split);
        Ok(n)
    }
}

impl std::io::Write for EnvoyStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            return Err(ErrorKind::BrokenPipe.into());
        }
        inner.write_buffer.put(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
