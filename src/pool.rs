use std::sync::atomic::{AtomicUsize, Ordering};

// todo: make fields private and expose methods
pub struct UpstreamPool {
    pub upstreams: Vec<Upstream>,
    pub index: AtomicUsize,
}

type Upstream = String;

impl UpstreamPool {
    pub fn new(upstreams: Vec<String>) -> Self {
        UpstreamPool {
            upstreams,
            index: AtomicUsize::from(0),
        }
    }

    pub fn get_upstream(&self) -> &String {
        if self.upstreams.is_empty() {
            //todo: panic on parse of no servers are set
            panic!("Fatal Error: No upstream servers set")
        }
        let pool_size = self.upstreams.len();
        let current_index = self.index.load(Ordering::Relaxed);
        let current = &self.upstreams[current_index % pool_size];
        self.index.fetch_add(1, Ordering::Relaxed);
        current
    }
}
