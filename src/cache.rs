use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

use axum::{
    extract::Request as AxumRequest,
    http::{HeaderMap, Uri},
};
use bytes::Bytes;
use reqwest::Method;

use crate::config::SiteMapEntry;

pub struct CacheHandler {
    pub cache: Mutex<HashMap<RequestData, CacheData>>,
    pub sitemap: Vec<SiteMapEntry>,
    pub current_bytes: AtomicUsize,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheData {
    pub bytes: Bytes,
    pub res_headers: HeaderMap,
    pub last_access: Duration,
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
    Cached(CacheData),
    Public,
    Private,
}

impl CacheHandler {
    pub fn new(sitemap: Vec<SiteMapEntry>, max_memory_mb: u64) -> Self {
        CacheHandler {
            cache: Mutex::new(HashMap::new()),
            sitemap,
            current_bytes: 0.into(),
            max_bytes: (max_memory_mb * 1024 * 1024) as usize,
        }
    }

    pub fn check(&self, req_data: &RequestData) -> PathType {
        let path = req_data.uri.path();

        let is_allowed = self.sitemap.iter().any(|entry| {
            let loc = &entry.loc;
            if loc == path {
                return true;
            }
            if loc.ends_with("/*") {
                let prefix = &loc[..loc.len() - 2];
                if path.starts_with(prefix) {
                    return true;
                }
            }
            false
        });

        if is_allowed || self.sitemap.is_empty() {
            let cached_response: Option<CacheData> = {
                let cache_guard = self.cache.lock().expect("Cache poisoned");
                (*cache_guard).get(req_data).cloned()
            };

            // todo: check and respect cache-control, age, etag, last-modified, etc... from upstream - RFC 9111
            match cached_response {
                Some(cache_data) => PathType::Cached(cache_data),
                None => PathType::Public,
            }
        } else {
            PathType::Private
        }
    }

    pub fn save(&self, req_data: RequestData, cache_data: CacheData) {
        let new_data_bytes = cache_data.bytes.len();
        let current_cache_bytes = self.current_bytes.load(Ordering::Relaxed);
        let max_cache_bytes = self.max_bytes;
        if new_data_bytes + current_cache_bytes < max_cache_bytes {
            let mut cache_guard = self.cache.lock().expect("Cache poisoned");
            // todo: find a better way to make this thread safe, this lock and double check is safe but inefficient
            if self.current_bytes.load(Ordering::Relaxed) + new_data_bytes < max_cache_bytes {
                self.current_bytes
                    .fetch_add(new_data_bytes, Ordering::Relaxed);
                (*cache_guard).insert(req_data, cache_data);
            }
        } else {
            // let new_data_priority = {
            //     self.sitemap.iter().find_map(|entry| {
            //         if entry.loc == req_data.uri.path() {
            //             return Some(entry.priority);
            //         }
            //         None
            //     });
            // };
            //
            // todo: weighted eviction
            eprintln!(
                "Cache full ({}MB). Skipping cache for: {}",
                self.max_bytes / (1024 * 1024),
                req_data.uri.path()
            );
        }
    }
}
