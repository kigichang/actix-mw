#[cfg(feature="csrf")]
pub mod csrf;

use std::{
    future::{ready, Ready, Future},
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

pub trait Handler {
    fn skip(&self, _: &ServiceRequest) -> bool {
        return false;
    }

    fn process(&self, req: ServiceRequest) -> Either<ServiceResponse, ServiceRequest>;
    fn post(&self, resp: ServiceResponse) -> ServiceResponse {
        return resp;
    }
}

pub struct Factory<T>
where
    T: Handler
{
    inner: Rc<T>,
    //_body: PhantomData<B>,
}

impl <T> Factory<T>
where
    T: Handler
{
    pub fn new(h: T) -> Self {
        Factory {
            inner: Rc::new(h),
        }
    }
}

impl<S, T> Transform<S, ServiceRequest> for Factory<T>
where
    S: Service<ServiceRequest, Response = ServiceResponse, Error = Error>,
    S::Future: 'static,
    T: Handler,
{
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Transform = Middleware<S, T>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(Middleware { 
            service,
            inner: self.inner.clone(),
        }))
    }
}

pub struct Middleware<S, T>
where
    T: Handler,
{
    service: S,
    inner: Rc<T>,
}

impl<S, T> Service<ServiceRequest> for Middleware<S, T>
where
    S: Service<ServiceRequest, Response = ServiceResponse, Error = Error>,
    S::Future: 'static,
    T: Handler,
{
    type Response = ServiceResponse;
    type Error = Error;
    type Future = HandlerFuture<S::Future, T>;

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
    pub enum HandlerFuture<Fut, T>
    where
        Fut: Future,
        T: Handler,
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
            fut: LocalBoxFuture<'static, Result<ServiceResponse, Error>>,
            inner: Rc<T>,
        },
    }
}

impl<Fut, T> Future for HandlerFuture<Fut, T>
where
    Fut: Future<Output = Result<ServiceResponse, Error>>,
    T: Handler,
{
    type Output = Result<ServiceResponse, Error>;

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


pub(crate) fn match_uri(uri: &str, check: &str) -> bool {
    if uri == check {
        return true
    }

    let mut check_with_right_boundary = check.to_string();
    check_with_right_boundary.push('/');
    return uri.starts_with(&check_with_right_boundary);
}

#[cfg(test)]
mod tests {
    use sha2::Digest;
    #[test]
    fn test_hash() {
        let salt = "test".to_string();
        let now = chrono::Utc::now().timestamp_millis();
        let now = now.to_le_bytes();
        let mut src = Vec::with_capacity(8 + salt.len());
        src.extend_from_slice(&now);
        src.extend_from_slice(&salt.as_bytes());

        let hash = sha2::Sha256::digest(&src);
        println!("{:?}", hex::encode(hash));
    }
}