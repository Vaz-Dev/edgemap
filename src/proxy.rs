use axum::{body::{Body as AxumBody}, extract::Request as AxumRequest, response::Response as AxumResponse };
use http_body_util::BodyExt;
use reqwest::{Client, header::ValueDrain};
use std::{ sync::{Arc, Mutex}, time::Duration};
use tower::{limit::ConcurrencyLimitLayer};

use crate::pool::ServerPool;

pub struct ProxyHandler {
    client: Client,
    pool: Arc<Mutex<ServerPool>>,
}

impl ProxyHandler {
    pub fn new(pool: ServerPool) -> Self {
            let client = Client::builder()
                .timeout(Duration::from_millis(5000))
                .connect_timeout(Duration::from_millis(200))
                .connector_layer(tower::timeout::TimeoutLayer::new(Duration::from_millis(50)))
                .connector_layer(ConcurrencyLimitLayer::new(10))
                .build()
                .unwrap();
            ProxyHandler {
                client,
                pool: Arc::new(Mutex::new(pool))
            }
    }

    pub async fn handle(&self, axum_req: AxumRequest<AxumBody>) -> Result<AxumResponse, Box<dyn std::error::Error + Send + Sync>> {

        let server = {
            // todo: recover from poisoned mutex
            let mut pool_guard = self.pool.lock().expect("Pool poisoned");
            pool_guard.direct_and_rotate()
        };

        let method = axum_req.method().clone();
        let uri = axum_req.uri().clone();
        let headers = axum_req.headers().clone();
        let body = axum_req.into_body().collect().await?.to_bytes();

        let server_res = self.client
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
        let res_body = server_res.bytes().await?;
        let proxy_res = res_builder
            .body(AxumBody::from(res_body))?;

        Ok(proxy_res)
    }
}
