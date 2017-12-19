//! A Proxy Connector crate for Hyper based applications
//!
//! # Example
//! ```rust,no_run
//! extern crate hyper;
//! extern crate hyper_proxy;
//! extern crate futures;
//! extern crate tokio_core;
//!
//! use hyper::{Chunk, Client};
//! use hyper::client::HttpConnector;
//! use hyper::header::Basic;
//! use futures::{Future, Stream};
//! use hyper_proxy::{Proxy, Intercept};
//! use tokio_core::reactor::Core;
//!
//! fn main() {
//!     let mut core = Core::new().unwrap();
//!     let handle = core.handle();
//!
//!     let proxy = {
//!         let proxy_uri = "http://my-proxy:8080".parse().unwrap();
//!         let proxy_connector = HttpConnector::new(4, &handle);
//!         let mut proxy = Proxy::new(proxy_connector, Intercept::All, proxy_uri).unwrap();
//!         proxy.set_authorization(Basic {
//!             username: "John Doe".into(),
//!             password: Some("Agent1234".into()),
//!         });
//!         proxy
//!     };
//!
//!     let client = Client::configure().connector(proxy).build(&handle);
//!     let uri = "http://my-remote-website.com".parse().unwrap();
//!     let fut = client
//!         .get(uri)
//!         .and_then(|res| res.body().concat2())
//!         .map(move |body: Chunk| ::std::str::from_utf8(&body).unwrap().to_string());
//!
//!     let res = core.run(fut).unwrap();
//! }
//! ```

#![deny(missing_docs)]

extern crate bytes;
#[macro_use]
extern crate futures;
extern crate hyper;
#[cfg(test)]
extern crate hyper_tls;
extern crate native_tls;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_tls;

mod tunnel;
mod stream;

use std::any::Any;
use std::fmt;
use std::io;
use std::sync::Arc;
use futures::Future;
use hyper::Uri;
use hyper::client::Service;
use hyper::header::{Header, Headers, Authorization, ProxyAuthorization, Scheme};
use native_tls::TlsConnector;
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_tls::TlsConnectorExt;
use stream::ProxyStream;

/// The Intercept enum to filter connections
#[derive(Debug, Clone)]
pub enum Intercept {
    /// All incoming connection will go through proxy
    All,
    /// Only http connections will go through proxy
    Http,
    /// Only https connections will go through proxy
    Https,
    /// No connection will go through this proxy
    None,
    /// A custom intercept
    Custom(Custom),
}

/// A Custom struct to proxy custom uris
#[derive(Clone)]
pub struct Custom(Arc<Fn(&Uri) -> bool + Send + Sync>);

impl fmt::Debug for Custom {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "_")
    }
}

impl<F: Fn(&Uri) -> bool + Send + Sync + 'static> From<F> for Custom {
    fn from(f: F) -> Custom {
        Custom(Arc::new(f))
    }
}

impl Intercept {
    /// A function to check if given `Uri` is proxied
    pub fn matches(&self, uri: &Uri) -> bool {
        match (self, uri.scheme()) {
            (&Intercept::All, _)
            | (&Intercept::Http, Some("http"))
            | (&Intercept::Https, Some("https")) => true,
            (&Intercept::Custom(Custom(ref f)), _) => f(uri),
            _ => false,
        }
    }
}

/// The proxy
#[derive(Clone)]
pub struct Proxy<C> {
    intercept: Intercept,
    headers: Headers,
    uri: Uri,
    connector: C,
    tls: Option<TlsConnector>,
}

impl<C: fmt::Debug> fmt::Debug for Proxy<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "Proxy {{ intercept: {:?}, headers: {:?}, uri: {:?}, connector: {:?} }}",
            self.intercept, self.headers, self.uri, self.connector
        )
    }
}

impl<C> Proxy<C> {
    /// Create a new secured Proxy
    pub fn new(connector: C, intercept: Intercept, uri: Uri) -> Result<Self, io::Error> {
        let tls = TlsConnector::builder()
            .and_then(|b| b.build())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(Proxy {
            intercept: intercept,
            uri: uri,
            headers: Headers::new(),
            connector: connector,
            tls: Some(tls),
        })
    }

    /// Create a new unsecured Proxy
    pub fn unsecured(connector: C, intercept: Intercept, uri: Uri) -> Self {
        Proxy {
            intercept: intercept,
            uri: uri,
            headers: Headers::new(),
            connector: connector,
            tls: None,
        }
    }

    /// Change proxy connector
    pub fn with_connector<CC>(self, connector: CC) -> Proxy<CC> {
        Proxy {
            intercept: self.intercept,
            uri: self.uri,
            headers: self.headers,
            connector: connector,
            tls: self.tls,
        }
    }

    /// Set proxy authorization
    pub fn set_authorization<S: Scheme + Any>(&mut self, scheme: S) {
        match self.intercept {
            Intercept::Http => self.headers.set(Authorization(scheme)),
            Intercept::Https => self.headers.set(ProxyAuthorization(scheme.clone())),
            _ => {
                self.headers.set(ProxyAuthorization(scheme.clone()));
                self.headers.set(Authorization(scheme));
            }
        }
    }

    /// Set a custom header
    pub fn set_header<H: Header>(&mut self, header: H) {
        self.headers.set(header);
    }

    /// Set a custom header
    pub fn set_tls(&mut self, tls: Option<TlsConnector>) {
        self.tls = tls;
    }

    /// Get current intercept
    pub fn intercept(&self) -> &Intercept {
        &self.intercept
    }

    /// Get current `Headers` which must be sent to proxy
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Get current intercept
    pub fn uri(&self) -> &Uri {
        &self.uri
    }
}

impl<C> Service for Proxy<C>
where
    C: Service<Request = Uri, Error = io::Error> + 'static,
    C::Future: 'static,
    <C::Future as Future>::Item: AsyncRead + AsyncWrite + 'static,
{
    type Request = Uri;
    type Response = ProxyStream<C::Response>;
    type Error = io::Error;
    type Future = Box<Future<Item = ProxyStream<C::Response>, Error = Self::Error>>;

    fn call(&self, uri: Uri) -> Self::Future {
        if self.intercept.matches(&uri) {
            if uri.scheme() == Some("https") {
                let host = uri.host().unwrap().to_owned();
                let port = uri.port().unwrap_or(443);
                let tunnel = tunnel::Tunnel::new(&host, port, &self.headers);
                let proxy_stream = self.connector
                    .call(self.uri.clone())
                    .and_then(move |io| tunnel.with_stream(io));
                match self.tls.as_ref() {
                    Some(tls) => {
                        let tls = tls.clone();
                        Box::new(
                            proxy_stream
                                .and_then(move |io| tls.connect_async(&host, io).map_err(io_err))
                                .map(|s| ProxyStream::Secured(s)),
                        )
                    }
                    None => Box::new(proxy_stream.map(|s| ProxyStream::Regular(s))),
                }
            } else {
                // without TLS, there is absolutely zero benefit from tunneling, as the proxy can
                // read the plaintext traffic. Thus, tunneling is just restrictive to the proxies
                // resources.
                Box::new(
                    self.connector
                        .call(self.uri.clone())
                        .map(|s| ProxyStream::Regular(s)),
                )
            }
        } else {
            Box::new(self.connector.call(uri).map(|s| ProxyStream::Regular(s)))
        }
    }
}

#[inline]
fn io_err<E: Into<Box<::std::error::Error + Send + Sync>>>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}
