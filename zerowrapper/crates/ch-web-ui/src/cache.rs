use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use topcoat::router::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use topcoat::router::{Request, Response, StatusCode};

/// Cache-Control for HTML pages: never reuse a stale copy in the browser.
const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");

/// A tower [`Layer`] that adds `Cache-Control: no-store` to HTML responses.
///
/// Hashed assets keep their immutable caching (set by the topcoat asset
/// server); cookies, credentials and sessions are unaffected — caching
/// headers only govern how the browser reuses an HTTP response.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoStoreHtmlLayer;

impl<S> tower::Layer<S> for NoStoreHtmlLayer {
    type Service = NoStoreHtml<S>;

    fn layer(&self, inner: S) -> Self::Service {
        NoStoreHtml { inner }
    }
}

/// Tower service applying the no-store policy to HTML responses.
#[derive(Debug, Clone)]
pub struct NoStoreHtml<S> {
    inner: S,
}

impl<S> tower::Service<Request> for NoStoreHtml<S>
where
    S: tower::Service<Request, Response = Response> + Clone,
{
    type Response = Response;
    type Error = S::Error;
    type Future = NoStoreHtmlFuture<S>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        NoStoreHtmlFuture {
            fut: Box::pin(self.inner.call(req)),
        }
    }
}

/// Response future that tags HTML responses with a no-store header.
pub struct NoStoreHtmlFuture<S>
where
    S: tower::Service<Request, Response = Response>,
{
    fut: Pin<Box<S::Future>>,
}

impl<S> Future for NoStoreHtmlFuture<S>
where
    S: tower::Service<Request, Response = Response>,
{
    type Output = std::result::Result<Response, S::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut().fut.as_mut().poll(cx) {
            Poll::Ready(Ok(mut resp)) => {
                if is_html(&resp) {
                    resp.headers_mut().insert(CACHE_CONTROL, NO_STORE);
                }
                Poll::Ready(Ok(resp))
            }
            other => other,
        }
    }
}

/// True when the response carries an HTML content type.
fn is_html(resp: &Response) -> bool {
    resp.status() != StatusCode::NOT_FOUND
        && resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| ct.starts_with("text/html") || ct.contains("application/xhtml+xml"))
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::Layer;

    #[test]
    fn html_response_gets_no_store() {
        let svc = NoStoreHtmlLayer.layer(ok_service());
        let resp = tower::ServiceExt::oneshot(svc, Request::new(().into()));
        let resp = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(resp)
            .unwrap();
        assert_eq!(resp.headers()[CACHE_CONTROL], "no-store");
    }

    fn ok_service() -> impl tower::Service<Request, Response = Response, Error = std::convert::Infallible> + Clone
    {
        tower::service_fn(move |_: Request| {
            let r = Response::builder()
                .header("content-type", "text/html; charset=utf-8")
                .status(200)
                .body(().into())
                .unwrap();
            async move { Ok::<_, std::convert::Infallible>(r) }
        })
    }
}