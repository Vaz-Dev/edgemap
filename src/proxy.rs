use axum::{body::Body as AxumBody, extract::Request as AxumRequest, http::{HeaderMap, Uri}, response::Response as AxumResponse };
use bytes::Bytes;
use http_body_util::BodyExt;
use reqwest::{Client, Method, Response as ReqwestResponse, StatusCode, header::CACHE_STATUS};
use std::{hash::{Hash, Hasher}, time::Duration};
use tower::{limit::ConcurrencyLimitLayer};

use crate::{cache::{CacheHandler, CacheItem, PathType, Weight}, config::Config, pool::{Upstream, UpstreamPool}};

pub struct ProxyHandler {
    http_client: Client,
    pool: UpstreamPool,
    cache_handler: CacheHandler,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestData {
    pub uri: Uri,
    pub method: Method,
    pub headers: HeaderMap
}
impl RequestData {
    pub fn extract(request: &AxumRequest) -> Self {
        RequestData {
            uri: request.uri().clone(),
            method: request.method().clone(),
        // todo: avoid cloning headers?
            headers: request.headers().clone(),
        }
    }
}
impl Hash for RequestData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.uri.hash(state);
        self.method.hash(state);
    }
}

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
            PathType::Cached(cache_data) => self.handle_cached(cache_data).await,
            PathType::Public(priority) => self.handle_public(axum_req, priority).await,
            PathType::Private => self.handle_private(axum_req).await,
        }
    }

    async fn handle_public(&self, axum_req: AxumRequest<AxumBody>, priority: Weight) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {

        let server = self.pool.get_upstream();
        println!("DEBUG::<Proxy> - Fetching {} from upstream {} and storing in cache", axum_req.uri(), server);


        let req_data = RequestData::extract(&axum_req);
        let req_body = axum_req.into_body().collect().await?.to_bytes();
        let (status, res_headers, res_body) = self.forward_request(&req_data, req_body, server).await?;
        self.cache_handler.try_save(req_data, res_body.clone(), res_headers.clone(), priority);
        mount_response(status, res_headers, res_body, "fwd=miss")
    }

    async fn handle_private(&self, axum_req: AxumRequest<AxumBody>) -> Result<AxumResponse<AxumBody>, Box<dyn std::error::Error + Send + Sync>> {

        let server = self.pool.get_upstream();
        println!("DEBUG::<Proxy> - Bypassing {} to upstream {}", axum_req.uri(), server);

        let req_data = RequestData::extract(&axum_req);
        let req_body = axum_req.into_body().collect().await?.to_bytes();
        let (status, res_headers, res_body) = self.forward_request(&req_data, req_body, server).await?;
        mount_response(status, res_headers, res_body, "fwd=bypass")
    }


    async fn handle_cached(&self, cache_data: CacheItem) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {
        println!("DEBUG::<Proxy> - Loaded {} bytes from cache memory", &cache_data.bytes.len());
        mount_response(StatusCode::OK, cache_data.headers, cache_data.bytes, "hit")
    }

    async fn forward_request(
        &self, 
        req_data: &RequestData, 
        body: Bytes, 
        server: &Upstream
    ) -> Result<(StatusCode, HeaderMap, Bytes), reqwest::Error> {
        let response = self.http_client
            .request(req_data.method.clone(), format!("{}{}", server, req_data.uri.path()))
            .headers(req_data.headers.clone())
            .body(body)
            .send()
            .await?;

        let status = response.status();
        let headers = response.headers().clone();
        let body_bytes = response.bytes().await?;

        Ok((status, headers, body_bytes))
    }
    
}


fn mount_response(status: StatusCode, headers: HeaderMap, body: Bytes, cache_status: &str) -> Result<AxumResponse<AxumBody>, Box<dyn std::error::Error + Send + Sync>> {
    let mut builder = AxumResponse::builder().status(status);
    // todo: filter headers like Connection, Transfer-Encoding as in RFC 7230
    for (key, value) in headers.iter() {
            builder = builder.header(key, value)
    }
    builder = builder.header(CACHE_STATUS, format!("EdgeMap; {}", cache_status));
    let mounted_response = builder
        .body(AxumBody::from(body))?;
    Ok(mounted_response)
}
