use std::{collections::HashMap, sync::Mutex};

use axum::{
    extract::Request as AxumRequest,
    http::{HeaderMap, Uri},
};
use bytes::Bytes;
use reqwest::Method;

use crate::config::{Config, SiteMapEntry};

pub struct CacheHandler {
    pub cache: Mutex<HashMap<RequestData, CacheData>>,
    pub sitemap: Vec<SiteMapEntry>,
    current_bytes: usize,
    max_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheData {
    pub bytes: Bytes,
    pub res_headers: HeaderMap,
    // todo: other cache-control rules
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

pub enum PathType {
    CachedPath(CacheData),
    PublicPath,
    PrivatePath,
}

impl CacheHandler {
    pub fn new(sitemap: Vec<SiteMapEntry>, max_memory_mb: u64) -> Self {
        CacheHandler {
            cache: Mutex::new(HashMap::new()),
            sitemap,
            current_bytes: 0,
            max_bytes: (max_memory_mb * 1024 * 1024) as usize,
        }
    }

    pub fn check(&self, req_data: &RequestData) -> PathType {
        // todo: check sitemap and possibly early return with PrivatePath
        let cached_response: Option<CacheData> = {
            let cache_guard = self.cache.lock().expect("Cache poisoned");
            (*cache_guard).get(req_data).cloned()
        };
        // todo: check and respect cache-control, age, etag, last-modified, etc... from upstream - RFC 9111
        match cached_response {
            Some(cache_data) => PathType::CachedPath(cache_data),
            None => PathType::PublicPath,
        }
    }

    pub fn save(&self, req_data: RequestData, cache_data: CacheData) {
        let mut cache_guard = self.cache.lock().expect("Cache poisoned");
        (*cache_guard).insert(req_data, cache_data);
    }
}
