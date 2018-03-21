//! A Proxy Connector crate for Hyper based applications
//!
//! # Example
//! ```rust,no_run
//! extern crate hyper;
//! extern crate hyper_proxy;
//! extern crate futures;
//! extern crate tokio_core;
//!
//! use hyper::{Chunk, Client, Request, Method, Uri};
//! use hyper::client::HttpConnector;
//! use hyper::header::Basic;
//! use futures::{Future, Stream};
//! use hyper_proxy::{Proxy, ProxyConnector, Intercept};
//! use tokio_core::reactor::Core;
//!
//! fn main() {
//!     let mut core = Core::new().unwrap();
//!     let handle = core.handle();
//!
//!     let proxy = {
//!         let proxy_uri = "http://my-proxy:8080".parse().unwrap();
//!         let mut proxy = Proxy::new(Intercept::All, proxy_uri);
//!         proxy.set_authorization(Basic {
//!                                    username: "John Doe".into(),
//!                                    password: Some("Agent1234".into()),
//!                                });
//!         let connector = HttpConnector::new(4, &handle);
//!         let proxy_connector = ProxyConnector::from_proxy(connector, proxy).unwrap();
//!         proxy_connector
//!     };
//!
//!     // Connecting to http will trigger regular GETs and POSTs.
//!     // We need to manually append the relevant headers to the request
//!     let uri: Uri = "http://my-remote-website.com".parse().unwrap();
//!     let mut req = Request::new(Method::Get, uri.clone());
//!     if let Some(headers) = proxy.http_headers(&uri) {
//!         req.headers_mut().extend(headers.iter());
//!         req.set_proxy(true);
//!     }
//!     let client = Client::configure().connector(proxy).build(&handle);
//!     let fut_http = client.request(req)
//!         .and_then(|res| res.body().concat2())
//!         .map(move |body: Chunk| ::std::str::from_utf8(&body).unwrap().to_string());
//!
//!     // Connecting to an https uri is straightforward (uses 'CONNECT' method underneath)
//!     let uri = "https://my-remote-websitei-secured.com".parse().unwrap();
//!     let fut_https = client
//!         .get(uri)
//!         .and_then(|res| res.body().concat2())
//!         .map(move |body: Chunk| ::std::str::from_utf8(&body).unwrap().to_string());
//!
//!     let futs = fut_http.join(fut_https);
//!
//!     let (_http_res, _https_res) = core.run(futs).unwrap();
//! }
//! ```

#![deny(missing_docs)]

extern crate bytes;
#[macro_use]
extern crate futures;
extern crate hyper;
#[cfg(test)]
extern crate hyper_tls;
#[cfg(feature = "tls")]
extern crate native_tls;
extern crate tokio_core;
extern crate tokio_io;
#[cfg(feature = "tls")]
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
use hyper::header::{Authorization, Header, Headers, ProxyAuthorization, Scheme};
#[cfg(feature = "tls")]
use native_tls::TlsConnector;
use tokio_io::{AsyncRead, AsyncWrite};
#[cfg(feature = "tls")]
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

impl<F: Fn(&Uri) -> bool + Send + Sync + 'static> From<F> for Intercept {
    fn from(f: F) -> Intercept {
        Intercept::Custom(f.into())
    }
}

/// A Proxy strcut
#[derive(Clone, Debug)]
pub struct Proxy {
    intercept: Intercept,
    headers: Headers,
    uri: Uri,
}

impl Proxy {
    /// Create a new `Proxy`
    pub fn new<I: Into<Intercept>>(intercept: I, uri: Uri) -> Proxy {
        Proxy {
            intercept: intercept.into(),
            uri: uri,
            headers: Headers::new(),
        }
    }

    /// Set `Proxy` authorization
    pub fn set_authorization<S: Scheme + Any>(&mut self, scheme: S) {
        match self.intercept {
            Intercept::Http => self.headers.set(Authorization(scheme)),
            Intercept::Https => self.headers.set(ProxyAuthorization(scheme)),
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

    /// Get current intercept
    pub fn intercept(&self) -> &Intercept {
        &self.intercept
    }

    /// Get current `Headers` which must be sent to proxy
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// Get proxy uri
    pub fn uri(&self) -> &Uri {
        &self.uri
    }
}

/// A wrapper around `Proxy`s with a connector.
#[derive(Clone)]
pub struct ProxyConnector<C> {
    proxies: Vec<Proxy>,
    connector: C,
    #[cfg(feature = "tls")]
    tls: Option<TlsConnector>,
    #[cfg(not(feature = "tls"))]
    tls: Option<()>,
}

impl<C: fmt::Debug> fmt::Debug for ProxyConnector<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "ProxyConnector {}{{ proxies: {:?}, connector: {:?} }}",
            if self.tls.is_some() {
                ""
            } else {
                "(unsecured)"
            },
            self.proxies,
            self.connector
        )
    }
}

impl<C> ProxyConnector<C> {
    /// Create a new secured Proxies
    #[cfg(feature = "tls")]
    pub fn new(connector: C) -> Result<Self, io::Error> {
        let tls = TlsConnector::builder()
            .and_then(|b| b.build())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(ProxyConnector {
            proxies: Vec::new(),
            connector: connector,
            tls: Some(tls),
        })
    }

    /// Create a new unsecured Proxy
    pub fn unsecured(connector: C) -> Self {
        ProxyConnector {
            proxies: Vec::new(),
            connector: connector,
            tls: None,
        }
    }

    /// Create a proxy connector and attach a particular proxy
    #[cfg(feature = "tls")]
    pub fn from_proxy(connector: C, proxy: Proxy) -> Result<Self, io::Error> {
        let mut c = ProxyConnector::new(connector)?;
        c.proxies.push(proxy);
        Ok(c)
    }

    /// Create a proxy connector and attach a particular proxy
    pub fn from_proxy_unsecured(connector: C, proxy: Proxy) -> Self {
        let mut c = ProxyConnector::unsecured(connector);
        c.proxies.push(proxy);
        c
    }

    /// Change proxy connector
    pub fn with_connector<CC>(self, connector: CC) -> ProxyConnector<CC> {
        ProxyConnector {
            connector: connector,
            proxies: self.proxies,
            tls: self.tls,
        }
    }

    /// Set or unset tls when tunneling
    #[cfg(feature = "tls")]
    pub fn set_tls(&mut self, tls: Option<TlsConnector>) {
        self.tls = tls;
    }

    /// Get the current proxies
    pub fn proxies(&self) -> &[Proxy] {
        &self.proxies
    }

    /// Add a new additional proxy
    pub fn add_proxy(&mut self, proxy: Proxy) {
        self.proxies.push(proxy);
    }

    /// Extend the list of proxies
    pub fn extend_proxies<I: IntoIterator<Item = Proxy>>(&mut self, proxies: I) {
        self.proxies.extend(proxies)
    }

    /// Get http headers for a matching uri
    ///
    /// These headers must be appended to the hyper Request for the proxy to work properly.
    /// This is needed only for http requests.
    pub fn http_headers(&self, uri: &Uri) -> Option<&Headers> {
        if uri.scheme() != Some("http") {
            return None;
        }
        self.match_proxy(uri).map(|p| &p.headers)
    }

    fn match_proxy(&self, uri: &Uri) -> Option<&Proxy> {
        self.proxies.iter().find(|p| p.intercept.matches(uri))
    }
}

impl<C> Service for ProxyConnector<C>
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
        if let Some(ref p) = self.match_proxy(&uri) {
            if uri.scheme() == Some("https") {
                let host = uri.host().unwrap().to_owned();
                let port = uri.port().unwrap_or(443);
                let tunnel = tunnel::Tunnel::new(&host, port, &p.headers);
                let proxy_stream = self.connector
                    .call(p.uri.clone())
                    .and_then(move |io| tunnel.with_stream(io));
                match self.tls.as_ref() {
                    #[cfg(feature = "tls")]
                    Some(tls) => {
                        let tls = tls.clone();
                        Box::new(
                            proxy_stream
                                .and_then(move |io| tls.connect_async(&host, io).map_err(io_err))
                                .map(|s| ProxyStream::Secured(s)),
                        )
                    },
                    #[cfg(not(feature = "tls"))]
                    Some(_) => panic!("hyper-proxy was not built with TLS support"),

                    None => Box::new(proxy_stream.map(|s| ProxyStream::Regular(s))),
                }
            } else {
                // without TLS, there is absolutely zero benefit from tunneling, as the proxy can
                // read the plaintext traffic. Thus, tunneling is just restrictive to the proxies
                // resources.
                Box::new(
                    self.connector
                        .call(p.uri.clone())
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
