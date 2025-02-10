use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::uri::{Authority, Scheme};
use http::{Error as HttpError, Request, Response};
use hyper::body::{Body as HttpBody, Incoming};
use hyper_util::client::legacy::connect::Connect;
use hyper_util::client::legacy::{Client, ResponseFuture};

use crate::rewrite::PathRewriter;
use crate::Error;

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

#[expect(clippy::module_name_repetitions)]
pub struct RevProxyFuture {
    inner: Result<ResponseFuture, Option<HttpError>>,
}

impl RevProxyFuture {
    pub(crate) fn new<C, B, Pr>(
        client: &Client<C, B>,
        req: Request<B>,
        scheme: &Scheme,
        authority: &Authority,
        path: &mut Pr,
    ) -> Self
    where
        C: Connect + Clone + Send + Sync + 'static,
        B: HttpBody + Send + Default + 'static + Unpin,
        B::Data: Send,
        B::Error: Into<BoxErr>,
        Pr: PathRewriter,
    {
        let mut builder = Request::builder().method(req.method()).uri(req.uri());

        for (key, value) in req.headers() {
            builder = builder.header(key, value);
        }

        let (_, body) = req.into_parts();

        let inner = builder
            .body(body)
            .and_then(|mut req| {
                path.rewrite_uri(&mut req, scheme, authority)?;

                Ok(client.request(req))
            })
            .map_err(Some);

        Self { inner }
    }
}

impl Future for RevProxyFuture {
    type Output = Result<Result<Response<Incoming>, Error>, Infallible>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.inner {
            Ok(fut) => match Future::poll(Pin::new(fut), cx) {
                Poll::Ready(res) => Poll::Ready(Ok(res.map_err(Error::RequestFailed))),
                Poll::Pending => Poll::Pending,
            },
            Err(e) => match e.take() {
                Some(e) => Poll::Ready(Ok(Err(Error::InvalidUri(e)))),
                None => unreachable!("RevProxyFuture::poll() is called after ready"),
            },
        }
    }
}
