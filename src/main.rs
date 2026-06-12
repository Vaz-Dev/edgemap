use std::{env, sync::Arc};

use axum::{Router, extract::{Request, State}, response::IntoResponse, routing::get};
use http_body_util::Collected;
use reqwest::StatusCode;

use crate::{config::Config, pool::UpstreamPool, proxy::ProxyHandler};

mod pool;
mod proxy;
mod cache;
mod config;

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
    println!("EdgeMap 0.1 - Initializing...");
    let args: Vec<String> = env::args().collect();
    let config: Config = Config::new(args);
    let output_url = format!("0.0.0.0:{}", config.output_port);
    let handler: Arc<ProxyHandler> = Arc::new(ProxyHandler::new(config));

    // todo: add route() instead of fallback()
    let app = Router::new()
        .fallback(handle_proxy)
        .with_state(handler);

    let listener = tokio::net::TcpListener::bind(output_url).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
