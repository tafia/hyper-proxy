# hyper-proxy

[![Travis Build Status](https://travis-ci.org/tafia/hyper-proxy.svg?branch=master)](https://travis-ci.org/tafia/hyper-proxy)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![crates.io](http://meritbadge.herokuapp.com/hyper-proxy)](https://crates.io/crates/hyper-proxy)

A proxy connector for [hyper][1] based applications.

[Documentation][3]

## Example

```rust
extern crate hyper;
extern crate hyper_proxy;
extern crate futures;
extern crate tokio_core;

use hyper::{Chunk, Client};
use hyper::client::HttpConnector;
use hyper::header::Basic;
use futures::{Future, Stream};
use hyper_proxy::{Proxy, Intercept};
use tokio_core::reactor::Core;

fn main() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();

    // create a proxy connector, authenticate etc ...
    let proxy = {
        let proxy_uri = "http://my-proxy:8080".parse().unwrap();
        let proxy_connector = HttpConnector::new(4, &handle);
        let mut proxy = Proxy::new(proxy_connector, Intercept::All, proxy_uri).unwrap();
        proxy.set_authorization(Basic {
            username: "John Doe".into(),
            password: Some("Agent1234".into()),
        });
        proxy
    };

    // use the connector in your hyper Client
    let client = Client::configure().connector(proxy).build(&handle);
    let uri = "http://my-remote-website.com".parse().unwrap();
    let fut = client
        .get(uri)
        .and_then(|res| res.body().concat2())
        .map(move |body: Chunk| ::std::str::from_utf8(&body).unwrap().to_string());

    let res = core.run(fut).unwrap();
}
```

## Credits

Large part of the code comes from [reqwest][2].
The core part as just been extracted and slightly enhanced.

 Main changes are:
- support for authentication
- add non secured tunneling
- add the possibility to add additional headers when connecting to the proxy
- remove `Custom` proxy as it is not clear yet if this is that useful

[1]: https://crates.io/crates/hyper
[2]: https://github.com/seanmonstar/reqwest
[3]: https://docs.rs/hyper-proxy
