use std::io::{self, Cursor};
use bytes::{BufMut, IntoBuf};
use futures::{Async, Future, Poll};
use hyper::header::Headers;
use tokio_io::{AsyncRead, AsyncWrite};

pub(crate) struct Tunnel<S> {
    buf: Cursor<Vec<u8>>,
    stream: Option<S>,
    state: TunnelState,
}

enum TunnelState {
    Writing,
    Reading,
}

impl<S> Tunnel<S> {
    /// Creates a new tunnel through proxy
    pub fn new(host: &str, port: u16, headers: &Headers) -> Tunnel<S> {
        let buf = format!(
            "\
            CONNECT {0}:{1} HTTP/1.1\r\n\
            Host: {0}:{1}\r\n\
            {2}\r\n
            \r\n\
            ",
            host, port, headers
        ).into_bytes();

        Tunnel {
            buf: buf.into_buf(),
            stream: None,
            state: TunnelState::Writing,
        }
    }

    pub fn with_stream(mut self, stream: S) -> Self {
        self.stream = Some(stream);
        self
    }
}

impl<S: AsyncRead + AsyncWrite + 'static> Future for Tunnel<S> {
    type Item = S;
    type Error = io::Error;

    fn poll(&mut self) -> Poll<S, Self::Error> {
        loop {
            if let TunnelState::Writing = self.state {
                let n = try_ready!(self.stream.as_mut().unwrap().write_buf(&mut self.buf));
                if !self.buf.has_remaining_mut() {
                    self.state = TunnelState::Reading;
                    self.buf.get_mut().truncate(0);
                } else if n == 0 {
                    return Err(tunnel_eof());
                }
            } else {
                let n = try_ready!(
                    self.stream
                        .as_mut()
                        .unwrap()
                        .read_buf(&mut self.buf.get_mut())
                );
                let read = &self.buf.get_ref()[..];
                if n == 0 {
                    return Err(tunnel_eof());
                } else if read.len() > 12 {
                    if read.starts_with(b"HTTP/1.1 200") || read.starts_with(b"HTTP/1.0 200") {
                        if read.ends_with(b"\r\n\r\n") {
                            return Ok(Async::Ready(self.stream.take().unwrap()));
                        }
                    // else read more
                    } else {
                        return Err(io::Error::new(io::ErrorKind::Other, "unsuccessful tunnel"));
                    }
                }
            }
        }
    }
}

#[inline]
fn tunnel_eof() -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "unexpected eof while tunneling",
    )
}
