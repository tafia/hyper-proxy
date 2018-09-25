# hyper-proxy

[![Travis Build Status](https://travis-ci.org/tafia/hyper-proxy.svg?branch=master)](https://travis-ci.org/tafia/hyper-proxy)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![crates.io](http://meritbadge.herokuapp.com/hyper-proxy)](https://crates.io/crates/hyper-proxy)

A proxy connector for [hyper][1] based applications.

[Documentation][3]

## Example

```rust,no_run
extern crate hyper;
extern crate http;
extern crate hyper_proxy;
extern crate futures;
extern crate tokio;
extern crate typed_headers;

use hyper::{Chunk, Client, Request, Method, Uri};
use hyper::client::HttpConnector;
use futures::{Future, Stream};
use hyper_proxy::{Proxy, ProxyConnector, Intercept};
use tokio::runtime::current_thread::Runtime;
use typed_headers::Credentials;

fn main() {
    let mut core = Runtime::new().unwrap();
    
    let proxy = {
        let proxy_uri = "http://my-proxy:8080".parse().unwrap();
        let mut proxy = Proxy::new(Intercept::All, proxy_uri);
        proxy.set_authorization(Credentials::basic("John Doe", "Agent1234").unwrap());
        let connector = HttpConnector::new(4);
        let proxy_connector = ProxyConnector::from_proxy(connector, proxy).unwrap();
        proxy_connector
    };

    // Connecting to http will trigger regular GETs and POSTs.
    // We need to manually append the relevant headers to the request
    let uri: Uri = "http://my-remote-website.com".parse().unwrap();
    let mut req = Request::get(uri.clone()).body(hyper::Body::from(vec![])).unwrap();
    if let Some(headers) = proxy.http_headers(&uri) {
        req.headers_mut().extend(headers.clone().into_iter());
    }
    let client = Client::builder().build(proxy);
    let fut_http = client.request(req)
        .and_then(|res| res.into_body().concat2())
        .map(move |body: Chunk| ::std::str::from_utf8(&body).unwrap().to_string());

    // Connecting to an https uri is straightforward (uses 'CONNECT' method underneath)
    let uri = "https://my-remote-websitei-secured.com".parse().unwrap();
    let fut_https = client
        .get(uri)
        .and_then(|res| res.into_body().concat2())
        .map(move |body: Chunk| ::std::str::from_utf8(&body).unwrap().to_string());

    let futs = fut_http.join(fut_https);

    let (_http_res, _https_res) = core.block_on(futs).unwrap();
}
```

## Credits

Large part of the code comes from [reqwest][2].
The core part as just been extracted and slightly enhanced.

 Main changes are:
- support for authentication
- add non secured tunneling
- add the possibility to add additional headers when connecting to the proxy

[1]: https://crates.io/crates/hyper
[2]: https://github.com/seanmonstar/reqwest
[3]: https://docs.rs/hyper-proxy
