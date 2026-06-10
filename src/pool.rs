pub struct ServerPool {
    pub list: Vec<Server>,
    pub next: usize,
}

pub struct Server {
    pub url: String,
}

impl ServerPool {
    pub fn new() -> Self {
        ServerPool {
            list: vec![],
            next: 0,
        }
    }

    pub fn add(&mut self, url: String) {
        self.list.push(Server::new(url));
    }

    pub fn direct_and_rotate(&mut self) -> String {
        let pool_size = self.list.len();
        let current_index = self.next;
        let current = &self.list[current_index].url;
        if pool_size == 0 {
            todo!("Default HTTP response for 'no servers set'")
        }
        match pool_size - 1 <= current_index {
            false => self.next += 1,
            true => self.next = 0,
        }
        String::from(current)
    }
}

impl Server {
    pub fn new(url: String) -> Self {
        Server { url }
    }
}
