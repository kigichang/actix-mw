use std::{
    future::{ready, Ready, Future}, 
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error,
};

use futures_util::future::Either;
use futures_core::{
    future::LocalBoxFuture,
    ready
};
use pin_project_lite::pin_project;

pub trait Handler<B> {
    fn skip(&self, _: &ServiceRequest) -> bool {
        return false;
    }

    fn process(&self, req: ServiceRequest) -> Either<ServiceResponse<B>, ServiceRequest>;
    fn post(&self, resp: ServiceResponse<B>) -> ServiceResponse<B> {
        return resp;
    }
}

pub struct Factory<T, B>
where
    T: Handler<B>
{
    inner: Rc<T>,
    _body: PhantomData<B>,
}

impl <T, B> Factory<T, B>
where
    T: Handler<B>
{
    pub fn new(h: T) -> Self {
        Factory {
            inner: Rc::new(h),
            _body: PhantomData,
        }
    }
}

impl<S, B, T> Transform<S, ServiceRequest> for Factory<T, B>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    T: Handler<B>,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = Middleware<S, B, T>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(Middleware { 
            service,
            inner: self.inner.clone(),
            _body: PhantomData,
        }))
    }
}

pub struct Middleware<S, B, T>
where
    T: Handler<B>,
{
    service: S,
    inner: Rc<T>,
    _body: PhantomData<B>,
}

impl<S, B, T> Service<ServiceRequest> for Middleware<S, B, T>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    T: Handler<B>,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = HandlerFuture<S::Future, B, T>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        if self.inner.skip(&req) {
            return HandlerFuture::SkipFuture { fut: self.service.call(req) };
        }

        match self.inner.process(req) {
            Either::Left(res) => {
                let h = self.inner.clone();
                HandlerFuture::ErrorHandlerFuture { 
                    fut: Box::pin(async move {
                        Ok(res)
                    }),
                    inner: h,
                }
            },
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
    pub enum HandlerFuture<Fut, B, T>
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

impl<Fut, B, T> Future for HandlerFuture<Fut, B, T>
where
    Fut: Future<Output = Result<ServiceResponse<B>, Error>>,
    T: Handler<B>,
{
    type Output = Result<ServiceResponse<B>, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project() {
            HandlerProj::SkipFuture {
                fut,
            } => {
                Poll::Ready(Ok(ready!(fut.poll(cx))?))
            },
            HandlerProj::HandlerFuture {
                fut,
                inner,
            } => {
                let res = ready!(fut.poll(cx))?;
                let res = inner.post(res);
                Poll::Ready(Ok(res))
            }
            HandlerProj::ErrorHandlerFuture { 
                fut,
                inner,
            } => {
                let res = ready!(fut.as_mut().poll(cx))?;
                let res = inner.post(res);
                Poll::Ready(Ok(res))
            },
        }
    }
}