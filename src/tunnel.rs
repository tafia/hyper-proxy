use bytes::{BufMut, IntoBuf};
use futures::{Async, Future, Poll};
use http::HeaderMap;
use hyper::client::connect::Connected;
use io_err;
use std::fmt::{self, Display, Formatter};
use std::io::{self, Cursor};
use tokio_io::{AsyncRead, AsyncWrite};

pub(crate) struct Tunnel<S> {
    buf: Cursor<Vec<u8>>,
    stream: Option<S>,
    connected: Option<Connected>,
    state: TunnelState,
}

#[derive(Debug)]
enum TunnelState {
    Writing,
    Reading,
}

struct HeadersDisplay<'a>(&'a HeaderMap);

impl<'a> Display for HeadersDisplay<'a> {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        for (key, value) in self.0 {
            let value_str = value.to_str().map_err(|_| fmt::Error)?;
            write!(f, "{}: {}\r\n", key.as_str(), value_str)?;
        }

        Ok(())
    }
}

impl<S> Tunnel<S> {
    /// Creates a new tunnel through proxy
    pub fn new(host: &str, port: u16, headers: &HeaderMap) -> Tunnel<S> {
        let buf = format!(
            "CONNECT {0}:{1} HTTP/1.1\r\n\
             Host: {0}:{1}\r\n\
             {2}\
             \r\n",
            host,
            port,
            HeadersDisplay(headers)
        ).into_bytes();

        Tunnel {
            buf: buf.into_buf(),
            stream: None,
            connected: None,
            state: TunnelState::Writing,
        }
    }

    /// Change stream
    pub fn with_stream(mut self, stream: S, connected: Connected) -> Self {
        self.stream = Some(stream);
        self.connected = Some(connected);
        self
    }
}

impl<S: AsyncRead + AsyncWrite + 'static> Future for Tunnel<S> {
    type Item = (S, Connected);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            if let TunnelState::Writing = self.state {
                let n = try_ready!(self.stream.as_mut().unwrap().write_buf(&mut self.buf));
                if !self.buf.has_remaining_mut() {
                    self.state = TunnelState::Reading;
                    self.buf.get_mut().truncate(0);
                } else if n == 0 {
                    return Err(io_err("unexpected EOF while tunnel writing"));
                }
            } else {
                let n = try_ready!(
                    self.stream
                        .as_mut()
                        .unwrap()
                        .read_buf(&mut self.buf.get_mut())
                );
                if n == 0 {
                    return Err(io_err("unexpected EOF while tunnel reading"));
                } else {
                    let read = &self.buf.get_ref()[..];
                    if read.len() > 12 {
                        if read.starts_with(b"HTTP/1.1 200") || read.starts_with(b"HTTP/1.0 200") {
                            if read.ends_with(b"\r\n\r\n") {
                                return Ok(Async::Ready((
                                    self.stream.take().unwrap(),
                                    self.connected.take().unwrap().proxy(true),
                                )));
                            }
                        // else read more
                        } else {
                            return Err(io_err("unsuccessful tunnel"));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate tokio;
    extern crate tokio_tcp;

    use self::tokio::runtime::current_thread::Runtime;
    use self::tokio_tcp::TcpStream;
    use super::{Connected, HeaderMap, Tunnel};
    use futures::Future;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn tunnel<S>(conn: S, host: String, port: u16) -> Tunnel<S> {
        Tunnel::new(&host, port, &HeaderMap::new()).with_stream(conn, Connected::new())
    }

    #[cfg_attr(rustfmt, rustfmt_skip)]
    macro_rules! mock_tunnel {
        () => {{
            mock_tunnel!(
                b"\
                HTTP/1.1 200 OK\r\n\
                \r\n\
                "
            )
        }};
        ($write:expr) => {{
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let connect_expected = format!(
                "\
                 CONNECT {0}:{1} HTTP/1.1\r\n\
                 Host: {0}:{1}\r\n\
                 \r\n\
                 ",
                addr.ip(),
                addr.port()
            ).into_bytes();

            thread::spawn(move || {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).unwrap();
                assert_eq!(&buf[..n], &connect_expected[..]);

                sock.write_all($write).unwrap();
            });
            addr
        }};
    }

    #[test]
    fn test_tunnel() {
        let addr = mock_tunnel!();

        let mut core = Runtime::new().unwrap();
        let work = TcpStream::connect(&addr);
        let host = addr.ip().to_string();
        let port = addr.port();
        let work = work.and_then(|tcp| tunnel(tcp, host, port));

        core.block_on(work).unwrap();
    }

    #[test]
    fn test_tunnel_eof() {
        let addr = mock_tunnel!(b"HTTP/1.1 200 OK");

        let mut core = Runtime::new().unwrap();
        let work = TcpStream::connect(&addr);
        let host = addr.ip().to_string();
        let port = addr.port();
        let work = work.and_then(|tcp| tunnel(tcp, host, port));

        core.block_on(work).unwrap_err();
    }

    #[test]
    fn test_tunnel_bad_response() {
        let addr = mock_tunnel!(b"foo bar baz hallo");

        let mut core = Runtime::new().unwrap();
        let work = TcpStream::connect(&addr);
        let host = addr.ip().to_string();
        let port = addr.port();
        let work = work.and_then(|tcp| tunnel(tcp, host, port));

        core.block_on(work).unwrap_err();
    }
}
