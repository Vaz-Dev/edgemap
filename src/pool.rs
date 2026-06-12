// todo: make fields private and expose methods
pub struct UpstreamPool {
    pub upstreams: Vec<Upstream>,
    pub next: usize,
}

type Upstream = String;

impl UpstreamPool {
    pub fn new(upstreams: Vec<String>) -> Self {
        UpstreamPool { upstreams, next: 0 }
    }

    pub fn direct_and_rotate(&mut self) -> String {
        if self.upstreams.is_empty() {
            //todo: panic on parse of no servers are set
            panic!("Fatal Error: No upstream servers set")
        }
        let pool_size = self.upstreams.len();
        let current_index = self.next;
        let current = &self.upstreams[current_index];
        match pool_size <= current_index + 1 {
            false => self.next += 1,
            true => self.next = 0,
        }
        String::from(current)
    }
}
