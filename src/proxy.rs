use axum::{body::Body as AxumBody, extract::Request as AxumRequest, http::{HeaderMap, Uri}, response::Response as AxumResponse };
use bytes::Bytes;
use http_body_util::BodyExt;
use reqwest::{Client, Method, Response as ReqwestResponse, StatusCode};
use std::{ sync::{Arc, Mutex}, time::Duration};
use tower::{limit::ConcurrencyLimitLayer};

use crate::{cache::{CacheData, CacheHandler, PathType, RequestData}, pool::ServerPool};

pub struct ProxyHandler {
    http_client: Client,
    pool: Arc<Mutex<ServerPool>>,
    cache_handler: CacheHandler,
}


impl ProxyHandler {
    pub fn new(pool: ServerPool, sitemap: Vec<String>) -> Self {
            let client = Client::builder()
                .timeout(Duration::from_millis(5000))
                .connect_timeout(Duration::from_millis(200))
                .connector_layer(tower::timeout::TimeoutLayer::new(Duration::from_millis(50)))
                .connector_layer(ConcurrencyLimitLayer::new(10))
                .build()
                .unwrap();
            ProxyHandler {
                http_client: client,
                pool: Arc::new(Mutex::new(pool)),
                cache_handler: CacheHandler::new(sitemap)
            }
    }

    pub async fn handle(&self, axum_req: AxumRequest<AxumBody>) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {
        let request_data = RequestData::extract(&axum_req);
        let path_type = self.cache_handler.check(&request_data);

        match path_type {
            PathType::CachedPath(cache_data) => self.handle_cached(cache_data, axum_req).await,
            PathType::PublicPath => self.handle_public(axum_req).await,
            PathType::PrivatePath => self.handle_private(axum_req).await,
        }
    }

    async fn handle_public(&self, axum_req: AxumRequest<AxumBody>) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {

        let server = {
            // todo: recover from poisoned mutex
            let mut pool_guard = self.pool.lock().expect("Pool poisoned");
            pool_guard.direct_and_rotate()
        };

        let req_data = RequestData::extract(&axum_req);
        // todo: avoid cloning headers?
        let body: Bytes = axum_req.into_body().collect().await?.to_bytes();

        let server_res: ReqwestResponse = self.http_client
            .request(req_data.method.clone(), format!("{}{}", server, req_data.uri.path()))
            .body(body)
            .send()
            .await?;

        let res_status = server_res.status();
        let mut res_builder = AxumResponse::builder().status(res_status);
        let res_headers = server_res.headers().clone();
        //todo: switch to axum::http::headerName
        for (key, value) in res_headers.iter() {
                res_builder = res_builder.header(key, value)
        }
        res_builder = res_builder.header("Cache-Status", "EdgeMap; fwd=miss");
        let res_body = server_res.bytes().await?;
        let cache_data = CacheData { bytes: res_body.clone(), res_headers };
        let proxy_res = res_builder
            .body(AxumBody::from(res_body))?;

        self.cache_handler.save(req_data, cache_data);
        Ok(proxy_res)
    }

    async fn handle_private(&self, axum_req: AxumRequest<AxumBody>) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {

        let server = {
            // todo: recover from poisoned mutex
            let mut pool_guard = self.pool.lock().expect("Pool poisoned");
            pool_guard.direct_and_rotate()
        };

        let method: Method = axum_req.method().clone();
        let uri: Uri = axum_req.uri().clone();
        // todo: avoid cloning headers?
        let headers: HeaderMap = axum_req.headers().clone();
        let body: Bytes = axum_req.into_body().collect().await?.to_bytes();

        let server_res: ReqwestResponse = self.http_client
            .request(method, format!("{}{}", server, uri.path()))
            .headers(headers)
            .body(body)
            .send()
            .await?;

        let res_status = server_res.status();
        let mut res_builder = AxumResponse::builder().status(res_status);
        //todo: switch to axum::http::headerName
        for (key, value) in server_res.headers() {
                res_builder = res_builder.header(key, value)
        }
        res_builder = res_builder.header("Cache-Status", "EdgeMap; fwd=bypass");
        let res_body = server_res.bytes().await?;
        let proxy_res = res_builder
            .body(AxumBody::from(res_body))?;

        Ok(proxy_res)
    }


    async fn handle_cached(&self, cache_data: CacheData, axum_req: AxumRequest<AxumBody>) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {

        let body_bytes: Bytes = cache_data.bytes;

        let res_status: StatusCode = StatusCode::OK;
        let mut res_builder = AxumResponse::builder().status(res_status);
        //todo: switch to axum::http::headerName
        for (key, value) in cache_data.res_headers.iter() {
                res_builder = res_builder.header(key, value)
        }
        res_builder = res_builder.header("Cache-Status", "EdgeMap; hit");
        let proxy_res = res_builder
            .body(AxumBody::from(body_bytes))?;

        Ok(proxy_res)
    }
    
}
