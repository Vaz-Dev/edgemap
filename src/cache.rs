use std::{
    collections::HashMap,
    sync::RwLock,
    time::{Duration, UNIX_EPOCH},
};

use axum::http::HeaderMap;
use bytes::Bytes;

use crate::{config::SiteMapEntry, proxy::RequestData};

// todo: make a clear() method that clears data/poisons
pub struct CacheHandler {
    state: RwLock<CacheState>,
    sitemap: Vec<SiteMapEntry>,
    max_bytes: usize,
}

struct CacheState {
    cache: HashMap<RequestData, CacheItem>,
    buckets: Vec<WeightsBucket>,
    current_bytes: usize,
}

// todo: make it Arc to avoid any deep copies
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheItem {
    pub bytes: Bytes,
    pub headers: HeaderMap,
    bucket_indexes: (usize, usize),
}

struct WeightsBucket {
    weight: Weight,
    current_bytes: usize,
    entries: Vec<WeightEntry>,
}

struct WeightEntry {
    key: RequestData,
    size_bytes: usize,
    created_at: Duration,
}

pub type Weight = f64;
pub enum PathType {
    Cached(CacheItem),
    Public(Weight),
    Private,
}

// todo: should not be a blacklist check, should be a whitelist
// todo: from upstream the cache-control: no-cache is different than no-store, it actually wants cache but for the proxy to always check for 304 Not Modified
// todo: same thing for max-age=0
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
                    current_bytes: 0,
                })
                .collect()
        };
        let mut sorted_sitemap = sitemap;
        sorted_sitemap.sort_by(|a, b| {
            b.priority
                .partial_cmp(&a.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let state = CacheState {
            cache: HashMap::new(),
            buckets,
            current_bytes: 0,
        };
        CacheHandler {
            state: RwLock::new(state),
            sitemap: sorted_sitemap,
            max_bytes: (max_memory_mb * 1024 * 1024) as usize,
        }
    }

    /// checks if the route is private, public or cached
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

        let is_allowed: Option<Weight> = self.sitemap.iter().find_map(|entry| {
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

    /// attempts to save an entry in the cache, possibly evicting another one or getting rejected
    pub fn try_save(
        &self,
        req_data: RequestData,
        body: Bytes,
        headers: HeaderMap,
        priority: Weight,
    ) {
        if let Some(cache_control_data) = headers.get(header::CACHE_CONTROL) {
            for blacklisted_value in CACHE_CONTROL_CONTAINS_BLACKLIST {
                if cache_control_data
                    .to_str()
                    .unwrap_or_default()
                    .contains(blacklisted_value)
                {
                    println!(
                        "DEBUG::<Cache> - Upstream endpoint {} requested not to cache this asset, using Cache-Control: {}",
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
            let can_alloc: bool = self.evict(body.len(), priority);
            if can_alloc {
                println!(
                    "DEBUG::<Cache> - Successfully evicted enough memory for {}, storing in cache",
                    req_data.uri
                );
                self.insert(req_data, body, headers, priority);
            } else {
                println!(
                    "DEBUG::<Cache> - Cache full ({}MB). Skipping cache for: {}",
                    self.max_bytes / (1024 * 1024),
                    req_data.uri.path()
                );
            };
        }
    }

    /// actually inserts (or fails silently) entries in the cache
    fn insert(&self, req_data: RequestData, body: Bytes, headers: HeaderMap, priority: Weight) {
        let mut cache_write_guard = self.state.write().expect("Cache poisoned");
        // todo: find a better way to make this thread safe AND fast, this lock and double check is safe but inefficient, maybe parking_lot's RwLock?
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
                    bucket.current_bytes += body.len();
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

    /// attempts to evict lower weighted entries to make space for the current entry
    /// the return boolean represents if this space was successfully allocated or not
    pub fn evict(&self, current_size_bytes: usize, current_weight: Weight) -> bool {
        let weights_lower_than_target: Vec<Weight> = self
            .sitemap
            .iter()
            .filter_map(|entry| {
                if entry.priority < current_weight {
                    Some(entry.priority)
                } else {
                    None
                }
            })
            .collect();
        if weights_lower_than_target.is_empty() {
            return false;
        }
        let mut cache_write_guard = self.state.write().expect("Cache posioned");
        let buckets = &cache_write_guard.buckets;
        let evictable_bytes: usize = buckets
            .iter()
            .filter(|bucket| bucket.weight < current_weight)
            .fold(0, |acc, bucket| acc + bucket.current_bytes);
        if evictable_bytes < current_size_bytes {
            return false;
        }
        let mut available_space_bytes = self.max_bytes - cache_write_guard.current_bytes;
        while available_space_bytes < current_size_bytes {
            let evicted_bytes: usize = {
                let target_bucket = cache_write_guard
                    .buckets
                    .iter_mut()
                    .filter(|bucket| !bucket.entries.is_empty() && bucket.weight < current_weight)
                    .min_by(|prev, next| {
                        prev.weight
                            .partial_cmp(&next.weight)
                            .expect("FATAL::<Cache> - Weight created from Sitemap Priority is NaN")
                    })
                    .expect("FATAL::<Cache> - Desync: Should be at least one bucket here");
                let evicted_item = target_bucket
                    .entries
                    .pop()
                    .expect("FATAL::<Cache> - Desync: Bucket should have at least one item");
                // todo: replace these panics with calls to reset all cache data and log the error
                let evicted_bytes = evicted_item.size_bytes;
                target_bucket.current_bytes -= evicted_bytes;
                cache_write_guard.cache.remove(&evicted_item.key);
                cache_write_guard.current_bytes -= evicted_bytes;
                println!(
                    "DEBUG::<Cache> - Evicted {} - Freeing {} bytes",
                    evicted_item.key.uri, evicted_bytes
                );
                evicted_bytes
            };
            available_space_bytes += evicted_bytes;
        }
        true
    }
}
