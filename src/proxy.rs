use axum::{body::Body as AxumBody, extract::Request as AxumRequest, http::{HeaderMap, Uri}, response::Response as AxumResponse };
use bytes::Bytes;
use http_body_util::BodyExt;
use reqwest::{Client, Method, Response as ReqwestResponse, StatusCode, header::CACHE_STATUS};
use std::time::Duration;
use tower::{limit::ConcurrencyLimitLayer};

use crate::{cache::{CacheHandler, CacheItem, PathType, Weight}, config::Config, pool::UpstreamPool};

pub struct ProxyHandler {
    http_client: Client,
    pool: UpstreamPool,
    cache_handler: CacheHandler,
}


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestData {
    pub uri: Uri,
    pub method: Method,
}
impl RequestData {
    pub fn extract(request: &AxumRequest) -> Self {
        RequestData {
            uri: request.uri().clone(),
            method: request.method().clone(),
        }
    }
}


// todo: extract common logic from the methods, too much DRY violations
impl ProxyHandler {
    pub fn new(config: Config) -> Self {
            let pool: UpstreamPool = UpstreamPool::new(config.upstreams);
            let client = Client::builder()
                .timeout(Duration::from_millis(5000))
                .connect_timeout(Duration::from_millis(200))
                .connector_layer(tower::timeout::TimeoutLayer::new(Duration::from_millis(50)))
                .connector_layer(ConcurrencyLimitLayer::new(10))
                .build()
                .unwrap();
            ProxyHandler {
                http_client: client,
                pool,
                cache_handler: CacheHandler::new(config.sitemap, config.max_memory_mb)
            }
    }

    pub async fn handle(&self, axum_req: AxumRequest<AxumBody>) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {
        let request_data = RequestData::extract(&axum_req);
        let path_type = self.cache_handler.check(&request_data, axum_req.headers());

        match path_type {
            PathType::Cached(cache_data) => self.handle_cached(cache_data, axum_req).await,
            PathType::Public(priority) => self.handle_public(axum_req, priority).await,
            PathType::Private => self.handle_private(axum_req).await,
        }
    }

    async fn handle_public(&self, axum_req: AxumRequest<AxumBody>, priority: Weight) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {

        let server = self.pool.get_upstream();
        println!("DEBUG::<Proxy> - Fetching {} from upstream {} and storing in cache", axum_req.uri(), server);

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
        res_builder = res_builder.header(CACHE_STATUS, "EdgeMap; fwd=miss");
        let res_body = server_res.bytes().await?;
        let proxy_res = res_builder
            .body(AxumBody::from(res_body.clone()))?;

        self.cache_handler.try_save(req_data, res_body, res_headers, priority);
        Ok(proxy_res)
    }

    async fn handle_private(&self, axum_req: AxumRequest<AxumBody>) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {

        let server = self.pool.get_upstream();
        println!("DEBUG::<Proxy> - Bypassing {} to upstream {}", axum_req.uri(), server);

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
        res_builder = res_builder.header(CACHE_STATUS, "EdgeMap; fwd=bypass");
        let res_body = server_res.bytes().await?;
        let proxy_res = res_builder
            .body(AxumBody::from(res_body))?;

        Ok(proxy_res)
    }


    async fn handle_cached(&self, cache_data: CacheItem, axum_req: AxumRequest<AxumBody>) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {

        let body_bytes: Bytes = cache_data.bytes;
        println!("DEBUG::<Proxy> - Loaded {} bytes from cache memory", body_bytes.len());

        let res_status: StatusCode = StatusCode::OK;
        let mut res_builder = AxumResponse::builder().status(res_status);
        //todo: switch to axum::http::headerName
        for (key, value) in cache_data.headers.iter() {
                res_builder = res_builder.header(key, value)
        }
        res_builder = res_builder.header(CACHE_STATUS, "EdgeMap; hit");
        let proxy_res = res_builder
            .body(AxumBody::from(body_bytes))?;

        Ok(proxy_res)
    }
    
}
