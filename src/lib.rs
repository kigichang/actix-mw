#[cfg(feature = "csrf")]
pub mod csrf;

use std::{
    future::{ready, Future, Ready},
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error,
};

use futures_core::{future::LocalBoxFuture, ready};
use futures_util::future::Either;
use pin_project_lite::pin_project;

pub trait Handler<B> {
    fn skip(&self, _: &ServiceRequest) -> bool {
        false
    }

    fn process(&self, req: ServiceRequest) -> Either<ServiceResponse<B>, ServiceRequest>;
    fn post(&self, resp: ServiceResponse<B>) -> ServiceResponse<B> {
        resp
    }
}

pub struct Factory<T, B>
where
    T: Handler<B>,
{
    inner: Rc<T>,
    _phantom: PhantomData<B>,
}

impl<T, B> Factory<T, B>
where
    T: Handler<B>,
{
    pub fn new(h: T) -> Self {
        Factory {
            inner: Rc::new(h),
            _phantom: PhantomData,
        }
    }
}

impl<S, T, B> Transform<S, ServiceRequest> for Factory<T, B>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    T: Handler<B>,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = Middleware<S, T, B>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(Middleware {
            service,
            inner: self.inner.clone(),
            _phantom: PhantomData,
        }))
    }
}

pub struct Middleware<S, T, B>
where
    T: Handler<B>,
{
    service: S,
    inner: Rc<T>,
    _phantom: PhantomData<B>,
}

impl<S, T, B> Service<ServiceRequest> for Middleware<S, T, B>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    T: Handler<B>,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = HandlerFuture<S::Future, T, B>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        if self.inner.skip(&req) {
            return HandlerFuture::SkipFuture {
                fut: self.service.call(req),
            };
        }

        match self.inner.process(req) {
            Either::Left(res) => {
                let h = self.inner.clone();
                HandlerFuture::ErrorHandlerFuture {
                    fut: Box::pin(async move { Ok(res) }),
                    inner: h,
                }
            }
            Either::Right(req) => {
                let h = self.inner.clone();

                HandlerFuture::HandlerFuture {
                    fut: self.service.call(req),
                    inner: h,
                }
            }
        }
    }
}

pin_project! {
    #[project = HandlerProj]
    pub enum HandlerFuture<Fut, T, B>
    where
        Fut: Future,
        T: Handler<B>,
    {
        SkipFuture {
            #[pin]
            fut: Fut,
        },

        HandlerFuture {
            #[pin]
            fut: Fut,
            inner: Rc<T>,
        },
        ErrorHandlerFuture {
            fut: LocalBoxFuture<'static, Result<ServiceResponse<B>, Error>>,
            inner: Rc<T>,
        },
    }
}

impl<Fut, T, B> Future for HandlerFuture<Fut, T, B>
where
    Fut: Future<Output = Result<ServiceResponse<B>, Error>>,
    T: Handler<B>,
{
    type Output = Result<ServiceResponse<B>, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project() {
            HandlerProj::SkipFuture { fut } => Poll::Ready(Ok(ready!(fut.poll(cx))?)),
            HandlerProj::HandlerFuture { fut, inner } => {
                let res = ready!(fut.poll(cx))?;
                let res = inner.post(res);
                Poll::Ready(Ok(res))
            }
            HandlerProj::ErrorHandlerFuture { fut, inner } => {
                let res = ready!(fut.as_mut().poll(cx))?;
                let res = inner.post(res);
                Poll::Ready(Ok(res))
            }
        }
    }
}

pub fn match_uri(test_uri: &str, check: &str) -> bool {
    if test_uri == check {
        return true;
    }

    let mut check_with_right_boundary = check.to_string();
    check_with_right_boundary.push('/');
    return test_uri.starts_with(&check_with_right_boundary);
}
