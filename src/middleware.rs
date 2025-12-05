use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use futures::future::LocalBoxFuture;
use std::future::{ready, Ready};

use crate::auth::{verify_token, Claims};

pub struct AuthMiddleware;

impl<S, B> Transform<S, ServiceRequest> for AuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = AuthMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuthMiddlewareService { service }))
    }
}

pub struct AuthMiddlewareService<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for AuthMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let auth_header = req
            .headers()
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "));

        match auth_header {
            Some(token) => match verify_token(token) {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    let fut = self.service.call(req);
                    Box::pin(async move {
                        let res = fut.await?;
                        Ok(res.map_into_left_body())
                    })
                }
                Err(_) => {
                    let (req, _pl) = req.into_parts();
                    let response = HttpResponse::Unauthorized()
                        .json(serde_json::json!({
                            "error": "Invalid or expired token"
                        }))
                        .map_into_right_body();
                    Box::pin(async move { Ok(ServiceResponse::new(req, response)) })
                }
            },
            None => {
                let (req, _pl) = req.into_parts();
                let response = HttpResponse::Unauthorized()
                    .json(serde_json::json!({
                        "error": "Missing authorization token"
                    }))
                    .map_into_right_body();
                Box::pin(async move { Ok(ServiceResponse::new(req, response)) })
            }
        }
    }
}

// Middleware that only enforces auth if require_auth_for_reads setting is true
pub struct ReadAuthMiddleware;

impl<S, B> Transform<S, ServiceRequest> for ReadAuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = ReadAuthMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(ReadAuthMiddlewareService { service }))
    }
}

pub struct ReadAuthMiddlewareService<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for ReadAuthMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        // Check if the site_setting requires auth for reads
        let pool = req.app_data::<actix_web::web::Data<crate::DbPool>>().cloned();

        let require_auth = pool
            .as_ref()
            .map(|p| crate::config::require_auth_for_reads(p))
            .unwrap_or(false);

        if !require_auth {
            // Setting is false, allow the request through without auth
            let fut = self.service.call(req);
            return Box::pin(async move {
                let res = fut.await?;
                Ok(res.map_into_left_body())
            });
        }

        // Setting is true, enforce authentication
        let auth_header = req
            .headers()
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "));

        match auth_header {
            Some(token) => match verify_token(token) {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    let fut = self.service.call(req);
                    Box::pin(async move {
                        let res = fut.await?;
                        Ok(res.map_into_left_body())
                    })
                }
                Err(_) => {
                    let (req, _pl) = req.into_parts();
                    let response = HttpResponse::Unauthorized()
                        .json(serde_json::json!({
                            "error": "Invalid or expired token"
                        }))
                        .map_into_right_body();
                    Box::pin(async move { Ok(ServiceResponse::new(req, response)) })
                }
            },
            None => {
                let (req, _pl) = req.into_parts();
                let response = HttpResponse::Unauthorized()
                    .json(serde_json::json!({
                        "error": "Missing authorization token"
                    }))
                    .map_into_right_body();
                Box::pin(async move { Ok(ServiceResponse::new(req, response)) })
            }
        }
    }
}

// Extractor to get the current user's claims from the request
pub struct AuthUser(pub Claims);

impl actix_web::FromRequest for AuthUser {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(
        req: &actix_web::HttpRequest,
        _payload: &mut actix_web::dev::Payload,
    ) -> Self::Future {
        match req.extensions().get::<Claims>() {
            Some(claims) => ready(Ok(AuthUser(claims.clone()))),
            None => ready(Err(actix_web::error::ErrorUnauthorized(
                "User not authenticated",
            ))),
        }
    }
}
