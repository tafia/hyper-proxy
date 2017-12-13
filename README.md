# hyper-proxy

A proxy connector for hyper based applications.

## Credits

Large part of the code comes from [reqwest](https://github.com/seanmonstar/reqwest).
It just extract the core part so everyone can use it.

 Main changes are:
- support for authentication
- add non secured tunneling
- add the possibility to add additional headers when connecting to the proxy
- remove `Custom` proxy as it is not clear yet if this is that useful


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
    let uri = "http://my-proxy:8080".parse().unwrap();

    let proxy = {
        let proxy_connector = HttpConnector::new(4, &handle);
        let mut proxy = Proxy::new(proxy_connector, Intercept::All, uri).unwrap();
        proxy.set_authorization(Basic {
            username: "John Doe".into(),
            password: Some("Agent1234".into()),
        });
        proxy
    };

    let client = Client::configure().connector(proxy).build(&handle);
    let uri = "http://my-remote-website.com".parse().unwrap();
    let fut = client
        .get(uri)
        .and_then(|res| res.body().concat2())
        .map(move |body: Chunk| ::std::str::from_utf8(&body).unwrap().to_string());

    let res = core.run(fut).unwrap();
}
```
