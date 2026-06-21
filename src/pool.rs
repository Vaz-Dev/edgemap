use std::sync::atomic::{AtomicUsize, Ordering};

pub struct UpstreamPool {
    upstreams: Vec<Upstream>,
    index: AtomicUsize,
}

pub type Upstream = String;

impl UpstreamPool {
    pub fn new(upstreams: Vec<String>) -> Self {
        UpstreamPool {
            upstreams,
            index: AtomicUsize::from(0),
        }
    }

    pub fn get_upstream(&self) -> &String {
        if self.upstreams.is_empty() {
            panic!("Fatal Error: No upstream servers set")
        }
        let pool_size = self.upstreams.len();
        let current_index = self.index.fetch_add(1, Ordering::Relaxed);
        &self.upstreams[current_index % pool_size]
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    fn create_test_empty_pool() -> UpstreamPool {
        UpstreamPool::new(vec![])
    }
    fn create_test_single_pool() -> UpstreamPool {
        let upstreams = vec![String::from("http://localhost:3000")];
        UpstreamPool::new(upstreams)
    }
    fn create_test_multiple_pool() -> UpstreamPool {
        let upstreams = vec![
            String::from("http://localhost:3000"),
            String::from("http://localhost:3001"),
            String::from("http://localhost:3002"),
            String::from("http://localhost:3003"),
            String::from("http://localhost:3004"),
        ];
        UpstreamPool::new(upstreams)
    }

    #[test]
    #[should_panic]
    fn get_from_empty_pool_panics() {
        create_test_empty_pool().get_upstream();
    }

    #[test]
    fn get_from_single_pool_repeats() {
        let pool = create_test_single_pool();
        assert_eq!(pool.get_upstream(), "http://localhost:3000");
        assert_eq!(pool.get_upstream(), "http://localhost:3000");
        assert_eq!(pool.get_upstream(), "http://localhost:3000");
    }

    #[test]
    fn get_from_multiple_pool_round_robin() {
        let pool = create_test_multiple_pool();
        assert_eq!(pool.get_upstream(), "http://localhost:3000");
        assert_eq!(pool.get_upstream(), "http://localhost:3001");
        assert_eq!(pool.get_upstream(), "http://localhost:3002");
        assert_eq!(pool.get_upstream(), "http://localhost:3003");
        assert_eq!(pool.get_upstream(), "http://localhost:3004");
        assert_eq!(pool.get_upstream(), "http://localhost:3000");
    }
}
