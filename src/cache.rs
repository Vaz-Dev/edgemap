use std::{
    collections::HashMap,
    sync::RwLock,
    time::{Duration, UNIX_EPOCH},
};

use axum::http::HeaderMap;
use bytes::Bytes;
use reqwest::header;

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
#[derive(Debug)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, Uri};
    use bytes::Bytes;
    use std::str::FromStr;

    fn create_test_cache(max_mb: u64) -> CacheHandler {
        let sitemap = vec![
            SiteMapEntry {
                loc: "/high/*".to_string(),
                priority: 1.0,
            },
            SiteMapEntry {
                loc: "/".to_string(),
                priority: 0.1,
            },
            SiteMapEntry {
                loc: "/low/*".to_string(),
                priority: 0.5,
            },
            SiteMapEntry {
                loc: "/repeated".to_string(),
                priority: 0.1,
            },
        ];
        CacheHandler::new(sitemap, max_mb)
    }

    fn make_req_data(path: &str) -> RequestData {
        RequestData::extract(
            &axum::extract::Request::builder()
                .uri(Uri::from_str(path).unwrap())
                .method(reqwest::Method::GET)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
    }

    #[test]
    fn test_bucket_uniqueness_and_ordering() {
        let cache = create_test_cache(1);

        assert_eq!(cache.state.read().unwrap().buckets.len(), 3);

        let weights: Vec<_> = cache
            .state
            .read()
            .unwrap()
            .buckets
            .iter()
            .map(|b| b.weight)
            .collect();
        assert!(weights.contains(&1.0));
        assert!(weights.contains(&0.5));
        assert!(weights.contains(&0.1));
    }

    #[test]
    fn test_check_exact_matching() {
        let cache = create_test_cache(1);

        let req = make_req_data("/low/file.txt");
        let result = cache.check(&req, &HeaderMap::new());
        if let PathType::Public(weight) = result {
            assert_eq!(weight, 0.5);
        } else {
            panic!("Expected Public path type");
        }
    }

    #[test]
    fn test_check_wildcard_matching() {
        let cache = create_test_cache(1);

        let req = make_req_data("/high/resource.json");
        let result = cache.check(&req, &HeaderMap::new());
        if let PathType::Public(weight) = result {
            assert_eq!(weight, 1.0);
        } else {
            panic!("Expected Public path type");
        }
    }

    #[test]
    fn test_check_private_route() {
        let cache = create_test_cache(1);

        let req = make_req_data("/api/users");
        let result = cache.check(&req, &HeaderMap::new());
        assert!(matches!(result, PathType::Private));
    }

    #[test]
    fn test_cache_blacklist_bypass() {
        let cache = create_test_cache(1);

        let bypass_headers = vec![
            "no-store",
            "no-cache",
            "max-age=0",
            "private",
            "no-store, no-cache",
            "public, max-age=0",
        ];

        for header_value in bypass_headers {
            let mut headers = HeaderMap::new();
            headers.insert(header::CACHE_CONTROL, header_value.parse().unwrap());

            let req = make_req_data("/high/page.html");
            let result = cache.check(&req, &headers);

            assert!(
                matches!(result, PathType::Private),
                "Header '{}' should bypass cache, but got {:?}",
                header_value,
                result
            );
        }
    }

    #[test]
    fn test_valid_caching_headers() {
        let cache = create_test_cache(1);

        let valid_headers = vec!["", "public", "public, max-age=3600", "must-revalidate"];

        for header_value in valid_headers {
            let mut headers = HeaderMap::new();
            if !header_value.is_empty() {
                headers.insert(header::CACHE_CONTROL, header_value.parse().unwrap());
            }

            let req = make_req_data("/high/page.html");
            let result = cache.check(&req, &headers);

            if let PathType::Private = result {
                panic!();
            }
        }
    }

    #[test]
    fn test_save_and_get_from_cache() {
        let cache = create_test_cache(1);

        let body = Bytes::from_static(b"edgemap");
        let req = make_req_data("/high/test");

        cache.try_save(req.clone(), body.clone(), HeaderMap::new(), 1.0);

        let result = cache.check(&req, &HeaderMap::new());
        match result {
            PathType::Cached(item) => {
                assert_eq!(item.bytes, body);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_eviction_removes_lower_priority() {
        let cache = create_test_cache(1);

        let low_body = Bytes::from(vec![0u8; 500 * 1024]); // 500KB
        let low_req = make_req_data("/low/large1");
        cache.try_save(low_req.clone(), low_body.clone(), HeaderMap::new(), 0.5);

        let low_body2 = Bytes::from(vec![1u8; 500 * 1024]);
        let low_req2 = make_req_data("/low/large2");
        cache.try_save(low_req2.clone(), low_body2.clone(), HeaderMap::new(), 0.5);

        let high_body = Bytes::from(vec![2u8; 600 * 1024]);
        let high_req = make_req_data("/high/new");
        cache.try_save(high_req.clone(), high_body.clone(), HeaderMap::new(), 1.0);

        let result = cache.check(&high_req, &HeaderMap::new());
        match result {
            PathType::Cached(item) => {
                assert_eq!(item.bytes, high_body);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_weighted_eviction_skips_same_or_higher_priority() {
        let cache = create_test_cache(1);

        let body0 = Bytes::from(vec![0u8; 800 * 1024]);
        let req0 = make_req_data("/high/item");
        cache.try_save(req0.clone(), body0.clone(), HeaderMap::new(), 1.0);

        let body1 = Bytes::from(vec![1u8; 500 * 1024]);
        let req1 = make_req_data("/high/newer");
        cache.try_save(req1.clone(), body1.clone(), HeaderMap::new(), 1.0);

        let body2 = Bytes::from(vec![2u8; 500 * 1024]);
        let req2 = make_req_data("/high/newer");
        cache.try_save(req2.clone(), body2.clone(), HeaderMap::new(), 0.5);

        let result0 = cache.check(&req0, &HeaderMap::new());
        match result0 {
            PathType::Cached(item) => {
                assert_eq!(item.bytes, body0);
            }
            _ => panic!(),
        }
        let result1 = cache.check(&req1, &HeaderMap::new());
        if let PathType::Cached(_) = result1 {
            panic!();
        }
        let result2 = cache.check(&req2, &HeaderMap::new());
        if let PathType::Cached(_) = result2 {
            panic!();
        }
    }

    #[test]
    fn test_max_bytes_limit() {
        let cache = create_test_cache(1);

        let body = Bytes::from(vec![0u8; 20 * 1024 * 1024]);
        let req = make_req_data("/huge");
        cache.try_save(req.clone(), body.clone(), HeaderMap::new(), 1.0);

        let result = cache.check(&req, &HeaderMap::new());
        if let PathType::Cached(_) = result {
            panic!();
        }
    }
}
