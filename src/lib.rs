//! A Proxy Connector crate for Hyper based applications
//!
//! # Example
//! ```rust,no_run
//! extern crate hyper;
//! extern crate http;
//! extern crate hyper_proxy;
//! extern crate futures;
//! extern crate tokio;
//! extern crate typed_headers;
//!
//! use hyper::{Chunk, Client, Request, Method, Uri};
//! use hyper::client::HttpConnector;
//! use futures::{Future, Stream};
//! use hyper_proxy::{Proxy, ProxyConnector, Intercept};
//! use tokio::runtime::current_thread::Runtime;
//! use typed_headers::Credentials;
//!
//! fn main() {
//!     let mut core = Runtime::new().unwrap();
//!
//!     let proxy = {
//!         let proxy_uri = "http://my-proxy:8080".parse().unwrap();
//!         let mut proxy = Proxy::new(Intercept::All, proxy_uri);
//!         proxy.set_authorization(Credentials::basic("John Doe", "Agent1234").unwrap());
//!         let connector = HttpConnector::new(4);
//!         let proxy_connector = ProxyConnector::from_proxy(connector, proxy).unwrap();
//!         proxy_connector
//!     };
//!
//!     // Connecting to http will trigger regular GETs and POSTs.
//!     // We need to manually append the relevant headers to the request
//!     let uri: Uri = "http://my-remote-website.com".parse().unwrap();
//!     let mut req = Request::get(uri.clone()).body(hyper::Body::from(vec![])).unwrap();
//!     if let Some(headers) = proxy.http_headers(&uri) {
//!         req.headers_mut().extend(headers.clone().into_iter());
//!     }
//!     let client = Client::builder().build(proxy);
//!     let fut_http = client.request(req)
//!         .and_then(|res| res.into_body().concat2())
//!         .map(move |body: Chunk| ::std::str::from_utf8(&body).unwrap().to_string());
//!
//!     // Connecting to an https uri is straightforward (uses 'CONNECT' method underneath)
//!     let uri = "https://my-remote-websitei-secured.com".parse().unwrap();
//!     let fut_https = client
//!         .get(uri)
//!         .and_then(|res| res.into_body().concat2())
//!         .map(move |body: Chunk| ::std::str::from_utf8(&body).unwrap().to_string());
//!
//!     let futs = fut_http.join(fut_https);
//!
//!     let (_http_res, _https_res) = core.block_on(futs).unwrap();
//! }
//! ```

#![deny(missing_docs)]

extern crate bytes;
#[macro_use]
extern crate futures;
extern crate http;
extern crate hyper;
#[cfg(feature = "rustls")]
extern crate hyper_rustls;
#[cfg(test)]
#[cfg(feature = "tls")]
extern crate hyper_tls;
#[cfg(feature = "tls")]
extern crate native_tls;
extern crate tokio_io;
#[cfg(feature = "rustls")]
extern crate tokio_rustls;
#[cfg(feature = "tls")]
extern crate tokio_tls;
extern crate typed_headers;
#[cfg(feature = "rustls")]
extern crate webpki;

mod stream;
mod tunnel;

use futures::Future;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use hyper::client::connect::{Connect, Connected, Destination};
use hyper::Uri;
#[cfg(feature = "tls")]
use native_tls::TlsConnector as NativeTlsConnector;
use std::fmt;
use std::io;
use std::sync::Arc;
use stream::ProxyStream;
#[cfg(feature = "rustls")]
use tokio_rustls::TlsConnector;
#[cfg(feature = "tls")]
use tokio_tls::TlsConnector;
use typed_headers::{Authorization, Credentials, HeaderMapExt, ProxyAuthorization};
#[cfg(feature = "rustls")]
use webpki::DNSNameRef;

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

/// A trait for matching between Destination and Uri
pub trait Dst {
    /// Returns the connection scheme, e.g. "http" or "https"
    fn scheme(&self) -> Option<&str>;
    /// Returns the host of the connection
    fn host(&self) -> Option<&str>;
    /// Returns the port for the connection
    fn port(&self) -> Option<u16>;
}

impl Dst for Uri {
    fn scheme(&self) -> Option<&str> {
        self.scheme_part().map(|s| s.as_str())
    }
    fn host(&self) -> Option<&str> {
        self.host()
    }
    fn port(&self) -> Option<u16> {
        self.port_u16()
    }
}

impl Dst for Destination {
    fn scheme(&self) -> Option<&str> {
        Some(self.scheme())
    }
    fn host(&self) -> Option<&str> {
        Some(self.host())
    }
    fn port(&self) -> Option<u16> {
        self.port()
    }
}

/// A Custom struct to proxy custom uris
#[derive(Clone)]
pub struct Custom(Arc<Fn(Option<&str>, Option<&str>, Option<u16>) -> bool + Send + Sync>);

impl fmt::Debug for Custom {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "_")
    }
}

impl<F: Fn(Option<&str>, Option<&str>, Option<u16>) -> bool + Send + Sync + 'static> From<F>
    for Custom
{
    fn from(f: F) -> Custom {
        Custom(Arc::new(f))
    }
}

impl Intercept {
    /// A function to check if given `Uri` is proxied
    pub fn matches<D: Dst>(&self, uri: &D) -> bool {
        match (self, uri.scheme()) {
            (&Intercept::All, _)
            | (&Intercept::Http, Some("http"))
            | (&Intercept::Https, Some("https")) => true,
            (&Intercept::Custom(Custom(ref f)), _) => f(uri.scheme(), uri.host(), uri.port()),
            _ => false,
        }
    }
}

impl<F: Fn(Option<&str>, Option<&str>, Option<u16>) -> bool + Send + Sync + 'static> From<F>
    for Intercept
{
    fn from(f: F) -> Intercept {
        Intercept::Custom(f.into())
    }
}

/// A Proxy strcut
#[derive(Clone, Debug)]
pub struct Proxy {
    intercept: Intercept,
    headers: HeaderMap,
    uri: Uri,
}

impl Proxy {
    /// Create a new `Proxy`
    pub fn new<I: Into<Intercept>>(intercept: I, uri: Uri) -> Proxy {
        Proxy {
            intercept: intercept.into(),
            uri: uri,
            headers: HeaderMap::new(),
        }
    }

    /// Set `Proxy` authorization
    pub fn set_authorization(&mut self, credentials: Credentials) {
        match self.intercept {
            Intercept::Http => {
                self.headers.typed_insert(&Authorization(credentials));
            }
            Intercept::Https => {
                self.headers.typed_insert(&ProxyAuthorization(credentials));
            }
            _ => {
                self.headers
                    .typed_insert(&Authorization(credentials.clone()));
                self.headers.typed_insert(&ProxyAuthorization(credentials));
            }
        }
    }

    /// Set a custom header
    pub fn set_header(&mut self, name: HeaderName, value: HeaderValue) {
        self.headers.insert(name, value);
    }

    /// Get current intercept
    pub fn intercept(&self) -> &Intercept {
        &self.intercept
    }

    /// Get current `Headers` which must be sent to proxy
    pub fn headers(&self) -> &HeaderMap {
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
    tls: Option<NativeTlsConnector>,
    #[cfg(feature = "rustls")]
    tls: Option<TlsConnector>,
    #[cfg(not(any(feature = "tls", feature = "rustls")))]
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
        let tls = NativeTlsConnector::builder()
            .build()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(ProxyConnector {
            proxies: Vec::new(),
            connector: connector,
            tls: Some(tls),
        })
    }

    /// Create a new secured Proxies
    #[cfg(feature = "rustls")]
    pub fn new(connector: C) -> Result<Self, io::Error> {
        let cfg = Arc::new(tokio_rustls::rustls::ClientConfig::new());
        let tls = TlsConnector::from(cfg);
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
    #[cfg(any(feature = "tls", feature = "rustls"))]
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
    #[cfg(any(feature = "tls"))]
    pub fn set_tls(&mut self, tls: Option<NativeTlsConnector>) {
        self.tls = tls;
    }

    /// Set or unset tls when tunneling
    #[cfg(any(feature = "rustls"))]
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
    pub fn http_headers(&self, uri: &Uri) -> Option<&HeaderMap> {
        if uri.scheme_part().map_or(true, |s| s.as_str() != "http") {
            return None;
        }
        self.match_proxy(uri).map(|p| &p.headers)
    }

    fn match_proxy<D: Dst>(&self, uri: &D) -> Option<&Proxy> {
        self.proxies.iter().find(|p| p.intercept.matches(uri))
    }
}

macro_rules! unwrap_or_future {
    ($expr:expr) => {
        match $expr {
            std::result::Result::Ok(val) => val,
            std::result::Result::Err(e) => return Box::new(futures::future::err(e)),
        }
    };
}

impl<C> Connect for ProxyConnector<C>
where
    C: Connect + 'static,
{
    type Transport = ProxyStream<C::Transport>;
    type Error = io::Error;
    type Future = Box<Future<Item = (Self::Transport, Connected), Error = Self::Error> + Send>;

    fn connect(&self, dst: Destination) -> Self::Future {
        if let Some(ref p) = self.match_proxy(&dst) {
            if dst.scheme() == "https" {
                let host = dst.host().to_owned();
                let port = dst.port().unwrap_or(443);
                let tunnel = tunnel::new(&host, port, &p.headers);

                let proxy_dst = unwrap_or_future!(proxy_dst(&dst, &p.uri));
                let proxy_stream = self
                    .connector
                    .connect(proxy_dst)
                    .map_err(io_err)
                    .and_then(move |(io, c)| tunnel.with_stream(io, c));
                match self.tls.as_ref() {
                    #[cfg(feature = "tls")]
                    Some(tls) => {
                        let tls = tls.clone();
                        Box::new(
                            proxy_stream
                                .and_then(move |(io, _)| {
                                    let tls = TlsConnector::from(tls);
                                    tls.connect(&host, io).map_err(io_err)
                                })
                                .map(|s| (ProxyStream::Secured(s), Connected::new().proxy(true))),
                        )
                    }
                    #[cfg(feature = "rustls")]
                    Some(tls) => {
                        let tls = tls.clone();
                        let dnsref = unwrap_or_future!(DNSNameRef::try_from_ascii_str(&host)
                            .map(|r| r.to_owned())
                            .map_err(|()| io_err_from_str("invalid hostname")));

                        Box::new(
                            proxy_stream
                                .and_then(move |(io, _)| {
                                    let tls = TlsConnector::from(tls);
                                    tls.connect(dnsref.as_ref(), io).map_err(io_err)
                                })
                                .map(|s| (ProxyStream::Secured(s), Connected::new().proxy(true))),
                        )
                    }
                    #[cfg(not(any(feature = "tls", feature = "rustls")))]
                    Some(_) => panic!("hyper-proxy was not built with TLS support"),

                    None => Box::new(proxy_stream.map(|(s, c)| (ProxyStream::Regular(s), c))),
                }
            } else {
                // without TLS, there is absolutely zero benefit from tunneling, as the proxy can
                // read the plaintext traffic. Thus, tunneling is just restrictive to the proxies
                // resources.
                let proxy_dst = unwrap_or_future!(proxy_dst(&dst, &p.uri));
                Box::new(
                    self.connector
                        .connect(proxy_dst)
                        .map_err(io_err)
                        .map(|(s, c)| (ProxyStream::Regular(s), Connected::new().proxy(true))),
                )
            }
        } else {
            Box::new(
                self.connector
                    .connect(dst)
                    .map_err(io_err)
                    .map(|(s, c)| (ProxyStream::Regular(s), c)),
            )
        }
    }
}

fn proxy_dst(dst: &Destination, proxy: &Uri) -> io::Result<Destination> {
    let mut dst = dst.clone();
    proxy
        .scheme_part()
        .map(|s| dst.set_scheme(s.as_str()).map_err(io_err))
        .unwrap_or_else(|| Err(io_err(format!("proxy uri missing scheme: {}", proxy))))?;
    proxy
        .host()
        .map(|h| dst.set_host(h).map_err(io_err))
        .unwrap_or_else(|| Err(io_err(format!("proxy uri missing host: {}", proxy))))?;
    dst.set_port(proxy.port_u16());

    Ok(dst)
}

#[inline]
fn io_err<E: Into<Box<::std::error::Error + Send + Sync>>>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}

#[inline]
fn io_err_from_str(e: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}
