# hyper-proxy

[![Travis Build Status](https://travis-ci.org/tafia/hyper-proxy.svg?branch=master)](https://travis-ci.org/tafia/hyper-proxy)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![crates.io](http://meritbadge.herokuapp.com/hyper-proxy)](https://crates.io/crates/hyper-proxy)

A proxy connector for [hyper][1] based applications.

[Documentation][3]

## Example

```rust,no_run
use hyper::{Client, Request, Uri};
use hyper::client::HttpConnector;
use futures::{TryFutureExt, TryStreamExt};
use hyper_proxy::{Proxy, ProxyConnector, Intercept};
use headers::Authorization;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let proxy = {
        let proxy_uri = "http://my-proxy:8080".parse().unwrap();
        let mut proxy = Proxy::new(Intercept::All, proxy_uri);
        proxy.set_authorization(Authorization::basic("John Doe", "Agent1234"));
        let connector = HttpConnector::new();
        let proxy_connector = ProxyConnector::from_proxy(connector, proxy).unwrap();
        proxy_connector
    };

    // Connecting to http will trigger regular GETs and POSTs.
    // We need to manually append the relevant headers to the request
    let uri: Uri = "http://my-remote-website.com".parse().unwrap();
    let mut req = Request::get(uri.clone()).body(hyper::Body::empty()).unwrap();

    if let Some(headers) = proxy.http_headers(&uri) {
        req.headers_mut().extend(headers.clone().into_iter());
    }

    let client = Client::builder().build(proxy);
    let fut_http = client.request(req)
        .and_then(|res| res.into_body().map_ok(|x|x.to_vec()).try_concat())
        .map_ok(move |body| ::std::str::from_utf8(&body).unwrap().to_string());

    // Connecting to an https uri is straightforward (uses 'CONNECT' method underneath)
    let uri = "https://my-remote-websitei-secured.com".parse().unwrap();
    let fut_https = client.get(uri)
        .and_then(|res| res.into_body().map_ok(|x|x.to_vec()).try_concat())
        .map_ok(move |body| ::std::str::from_utf8(&body).unwrap().to_string());

    let (http_res, https_res) = futures::future::join(fut_http, fut_https).await;
    let (_, _) = (http_res?, https_res?);

    Ok(())
}
```

## Features

`hyper-proxy` exposes three main Cargo features, to configure which TLS implementation it uses to
connect to a proxy. It can also be configured without TLS support, by compiling without default
features entirely. The supported list of configurations is:

1. No TLS support (`default-features = false`)
2. TLS support via `native-tls` to link against the operating system's native TLS implementation
   (default)
3. TLS support via `rustls` (`default-features = false, features = ["rustls"]`)
4. TLS support via `rustls`, using a statically-compiled set of CA certificates to bypass the
   operating system's default store (`default-features = false, features = ["rustls-webpki"]`)

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
