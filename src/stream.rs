use std::io::{self, Read, Write};
use bytes::{Buf, BufMut};
use futures::Poll;
use tokio_io::{AsyncRead, AsyncWrite};

#[cfg(feature = "tls")]
use tokio_tls::TlsStream;

/// A Proxy Stream wrapper
pub enum ProxyStream<R> {
    Regular(R),
    #[cfg(feature = "tls")]
    Secured(TlsStream<R>),
}

macro_rules! match_fn {
    ($self:expr, $fn:ident $(,$buf:expr)*) => {
        match *$self {
            ProxyStream::Regular(ref mut s) => s.$fn($($buf)*),
            #[cfg(feature = "tls")]
            ProxyStream::Secured(ref mut s) => s.$fn($($buf)*),
        }
    }
}

impl<R: Read + Write> Read for ProxyStream<R> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match_fn!(self, read, buf)
    }
}

impl<R: Read + Write> Write for ProxyStream<R> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match_fn!(self, write, buf)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        match_fn!(self, flush)
    }
}

impl<R: AsyncRead + AsyncWrite> AsyncRead for ProxyStream<R> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        match *self {
            ProxyStream::Regular(ref s) => s.prepare_uninitialized_buffer(buf),
            #[cfg(feature = "tls")]
            ProxyStream::Secured(ref s) => s.prepare_uninitialized_buffer(buf),
        }
    }

    fn read_buf<B: BufMut>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        match_fn!(self, read_buf, buf)
    }
}

impl<R: AsyncRead + AsyncWrite> AsyncWrite for ProxyStream<R> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        match_fn!(self, shutdown)
    }

    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        match_fn!(self, write_buf, buf)
    }
}
