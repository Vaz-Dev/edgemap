use axum::{body::Body as AxumBody, extract::Request as AxumRequest, response::Response as AxumResponse};
use reqwest::{Client, Error, Method, Url};
use std::{str::FromStr, time::Duration};
use tower::{limit::ConcurrencyLimitLayer};

use crate::pool::ServerPool;

pub struct ProxyHandler {
    client: Client,
    pool: ServerPool,
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
            ProxyHandler { client, pool }
    }

    pub async fn handle(mut self, axum_req: AxumRequest<AxumBody>) -> Result<AxumResponse, Error> {

        let method = axum_req.method().clone();
        let uri = axum_req.uri().clone();
        let headers = axum_req.headers().clone();
        
        let server = self.pool.direct_and_rotate();
        todo!();
    }
}
