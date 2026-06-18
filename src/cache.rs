use std::{
    collections::HashMap,
    sync::RwLock,
    time::{Duration, UNIX_EPOCH},
};

use axum::http::HeaderMap;
use bytes::Bytes;
use reqwest::header;

use crate::{config::SiteMapEntry, proxy::RequestData};

pub struct CacheHandler {
    pub state: RwLock<CacheState>,
    pub sitemap: Vec<SiteMapEntry>,
    pub max_bytes: usize,
}

struct CacheState {
    cache: HashMap<RequestData, CacheItem>,
    buckets: Vec<WeightsBucket>,
    current_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheItem {
    pub bytes: Bytes,
    pub headers: HeaderMap,
    pub bucket_indexes: (usize, usize),
}

pub struct WeightsBucket {
    pub weight: Priority,
    pub entries: Vec<WeightEntry>,
}

pub struct WeightEntry {
    key: RequestData,
    size_bytes: usize,
    created_at: Duration,
}

pub type Priority = f64;
pub enum PathType {
    Cached(CacheItem),
    Public(Priority),
    Private,
}

// todo: should not be a blacklist check, should be a whitelist
// todo: from upstream the cache-control: no-cache is different than no-store, it actually wants cache but for the proxy to always check for 304 Not Modified
static CACHE_CONTROL_CONTAINS_BLACKLIST: [&str; 4] =
    ["no-cache", "no-store", "max-age=0", "private"];

impl CacheHandler {
    pub fn new(sitemap: Vec<SiteMapEntry>, max_memory_mb: u64) -> Self {
        let buckets = {
            let mut weights = vec![];
            sitemap.iter().for_each(|entry| {
                let weight = entry.priority;
                if !weights.contains(&weight) {
                    weights.push(weight)
                }
            });
            weights
                .iter()
                .map(|weight| WeightsBucket {
                    weight: *weight,
                    entries: vec![],
                })
                .collect()
        };
        let state = CacheState {
            cache: HashMap::new(),
            buckets,
            current_bytes: 0,
        };
        CacheHandler {
            state: RwLock::new(state),
            sitemap,
            max_bytes: (max_memory_mb * 1024 * 1024) as usize,
        }
    }

    pub fn check(&self, req_data: &RequestData, headers: &HeaderMap) -> PathType {
        let path = req_data.uri.path();
        if let Some(cache_control_data) = headers.get(header::CACHE_CONTROL) {
            for blacklisted_value in CACHE_CONTROL_CONTAINS_BLACKLIST {
                if cache_control_data
                    .to_str()
                    .unwrap_or_default()
                    .contains(blacklisted_value)
                {
                    println!(
                        "DEBUG - Client requested bypass at {}, using header Cache-Control: {}",
                        req_data.uri, blacklisted_value
                    );
                    return PathType::Private;
                }
            }
        }

        let is_allowed: Option<Priority> = self.sitemap.iter().find_map(|entry| {
            let loc = &entry.loc;
            if loc == path {
                return Some(entry.priority);
            }
            if loc.ends_with("/*") {
                let prefix = &loc[..loc.len() - 2];
                if path.starts_with(prefix) {
                    return Some(entry.priority);
                }
            }
            None
        });

        if let Some(priority) = is_allowed {
            let cached_response: Option<CacheItem> = {
                let cache_read_guard = self.state.read().expect("Cache State Poisoned");
                cache_read_guard.cache.get(req_data).cloned()
            };

            // todo: check and respect cache-control, age, etag, last-modified, etc... from upstream - RFC 9111
            match cached_response {
                Some(cache_data) => PathType::Cached(cache_data),
                None => PathType::Public(priority),
            }
        } else {
            PathType::Private
        }
    }

    pub fn try_save(
        &self,
        req_data: RequestData,
        body: Bytes,
        headers: HeaderMap,
        priority: Priority,
    ) {
        if let Some(cache_control_data) = headers.get(header::CACHE_CONTROL) {
            for blacklisted_value in CACHE_CONTROL_CONTAINS_BLACKLIST {
                if cache_control_data
                    .to_str()
                    .unwrap_or_default()
                    .contains(blacklisted_value)
                {
                    println!(
                        "DEBUG - Upstream endpoint {} requested not to cache this asset, using Cache-Control: {}",
                        req_data.uri,
                        blacklisted_value
                    );
                    return;
                }
            }
        }
        let new_data_bytes = body.len();
        let current_cache_bytes = {
            let cache_read_guard = self.state.read().expect("Cache State Poisoned");
            cache_read_guard.current_bytes
        };
        let max_cache_bytes: usize = self.max_bytes;
        if new_data_bytes + current_cache_bytes < max_cache_bytes {
            self.insert(req_data, body, headers, priority);
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

    fn insert(&self, req_data: RequestData, body: Bytes, headers: HeaderMap, priority: Priority) {
        let mut cache_write_guard = self.state.write().expect("Cache poisoned");
        // todo: find a better way to make this thread safe AND fast, this lock and double check is safe but inefficient
        if cache_write_guard.current_bytes + body.len() < self.max_bytes {
            cache_write_guard.current_bytes += body.len();

            for (index, bucket) in cache_write_guard.buckets.iter_mut().enumerate() {
                if bucket.weight == priority {
                    bucket.entries.push(WeightEntry {
                        key: req_data.clone(),
                        size_bytes: body.len(),
                        created_at: UNIX_EPOCH.elapsed().unwrap(),
                    });
                    let bucket_indexes = (index, bucket.entries.len() - 1);
                    cache_write_guard.cache.insert(
                        req_data,
                        CacheItem {
                            bytes: body,
                            headers,
                            bucket_indexes,
                        },
                    );
                    break;
                }
            }
        }
    }
}
