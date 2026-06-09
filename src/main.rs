use axum::{Router, routing::get};

use crate::{pool::ServerPool, proxy::ProxyHandler};

mod pool;
mod proxy;

#[tokio::main]
async fn main() {
    let mut  pool = ServerPool::new();
    pool.add(String::from("localhost:3000"));
    let proxy_handler = ProxyHandler::new(pool);
}
