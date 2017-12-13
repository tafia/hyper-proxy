use std::io::{self, Read, Write};
use bytes::{Buf, BufMut};
use futures::Poll;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_tls::TlsStream;

/// A Proxy Stream wrapper
pub enum ProxyStream<R> {
    Regular(R),
    Secured(TlsStream<R>),
}

impl<R: Read + Write> Read for ProxyStream<R> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            ProxyStream::Regular(ref mut s) => s.read(buf),
            ProxyStream::Secured(ref mut s) => s.read(buf),
        }
    }
}

impl<R: Read + Write> Write for ProxyStream<R> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            ProxyStream::Regular(ref mut s) => s.write(buf),
            ProxyStream::Secured(ref mut s) => s.write(buf),
        }
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        match *self {
            ProxyStream::Regular(ref mut s) => s.flush(),
            ProxyStream::Secured(ref mut s) => s.flush(),
        }
    }
}

impl<R: AsyncRead + AsyncWrite> AsyncRead for ProxyStream<R> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        match *self {
            ProxyStream::Regular(ref s) => s.prepare_uninitialized_buffer(buf),
            ProxyStream::Secured(ref s) => s.prepare_uninitialized_buffer(buf),
        }
    }

    fn read_buf<B: BufMut>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        match *self {
            ProxyStream::Regular(ref mut s) => s.read_buf(buf),
            ProxyStream::Secured(ref mut s) => s.read_buf(buf),
        }
    }
}

impl<R: AsyncRead + AsyncWrite> AsyncWrite for ProxyStream<R> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        match *self {
            ProxyStream::Regular(ref mut s) => s.shutdown(),
            ProxyStream::Secured(ref mut s) => s.shutdown(),
        }
    }

    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        match *self {
            ProxyStream::Regular(ref mut s) => s.write_buf(buf),
            ProxyStream::Secured(ref mut s) => s.write_buf(buf),
        }
    }
}
