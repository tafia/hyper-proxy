use io_err;
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

#[derive(Debug)]
enum TunnelState {
    Writing,
    Reading,
}

impl<S> Tunnel<S> {
    /// Creates a new tunnel through proxy
    pub fn new(host: &str, port: u16, headers: &Headers) -> Tunnel<S> {
        let buf = format!(
            "CONNECT {0}:{1} HTTP/1.1\r\n\
             Host: {0}:{1}\r\n\
             {2}\
             \r\n",
            host, port, headers
        ).into_bytes();

        Tunnel {
            buf: buf.into_buf(),
            stream: None,
            state: TunnelState::Writing,
        }
    }

    /// Change stream
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
                                return Ok(Async::Ready(self.stream.take().unwrap()));
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use futures::Future;
    use tokio_core::reactor::Core;
    use tokio_core::net::TcpStream;
    use super::{Headers, Tunnel};

    fn tunnel<S>(conn: S, host: String, port: u16) -> Tunnel<S> {
        Tunnel::new(&host, port, &Headers::new()).with_stream(conn)
    }

    macro_rules! mock_tunnel {
        () => ({
            mock_tunnel!(b"\
                HTTP/1.1 200 OK\r\n\
                \r\n\
            ")
        });
        ($write:expr) => ({
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let connect_expected = format!("\
                CONNECT {0}:{1} HTTP/1.1\r\n\
                Host: {0}:{1}\r\n\
                \r\n\
            ", addr.ip(), addr.port()).into_bytes();

            thread::spawn(move || {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).unwrap();
                assert_eq!(&buf[..n], &connect_expected[..]);

                sock.write_all($write).unwrap();
            });
            addr
        })
    }

    #[test]
    fn test_tunnel() {
        let addr = mock_tunnel!();

        let mut core = Core::new().unwrap();
        let work = TcpStream::connect(&addr, &core.handle());
        let host = addr.ip().to_string();
        let port = addr.port();
        let work = work.and_then(|tcp| tunnel(tcp, host, port));

        core.run(work).unwrap();
    }

    #[test]
    fn test_tunnel_eof() {
        let addr = mock_tunnel!(b"HTTP/1.1 200 OK");

        let mut core = Core::new().unwrap();
        let work = TcpStream::connect(&addr, &core.handle());
        let host = addr.ip().to_string();
        let port = addr.port();
        let work = work.and_then(|tcp| tunnel(tcp, host, port));

        core.run(work).unwrap_err();
    }

    #[test]
    fn test_tunnel_bad_response() {
        let addr = mock_tunnel!(b"foo bar baz hallo");

        let mut core = Core::new().unwrap();
        let work = TcpStream::connect(&addr, &core.handle());
        let host = addr.ip().to_string();
        let port = addr.port();
        let work = work.and_then(|tcp| tunnel(tcp, host, port));

        core.run(work).unwrap_err();
    }
}
