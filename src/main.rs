use std::sync::Arc;

use axum::{Router, extract::{Request, State}, response::IntoResponse, routing::get};
use reqwest::StatusCode;

use crate::{pool::ServerPool, proxy::ProxyHandler};

mod pool;
mod proxy;
mod cache;

async fn handle_proxy(
    State(handler): State<Arc<ProxyHandler>>,
    req: Request,
) -> impl IntoResponse {
    match handler.handle(req).await {
        Ok(res) => res,
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Proxy Error: {}", e)).into_response(),
    }
}

#[tokio::main]
async fn main() {
    let mut  pool = ServerPool::new();
    // sitemap not implemented yet, but it needs to exist in the struct
    let sitemap = Vec::new();
    // todo: dynamic server pool from args?
    pool.add("http://localhost:3000".to_string());
    let handler = Arc::new(ProxyHandler::new(pool, sitemap));

    // todo: add route() instead of fallback()
    let app = Router::new()
        .fallback(handle_proxy)
        .with_state(handler);

    //todo: dynamic port from .env
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
