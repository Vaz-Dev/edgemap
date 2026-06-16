EdgeMap is a reverse proxy I built in Rust to explore high-performance edge caching and load balancing. The goal was to solve the single-thread bottleneck of Node.js applications by offloading static traffic to RAM while keeping the backend alive only for dynamic requests.

It acts as a smart layer between the client and a single/cluster of upstream servers, using a sitemap configuration to decide what gets cached.
Key Features

  - Async Reverse Proxy: Built on tokio and axum, handling concurrent requests without blocking.
  - Sitemap-Driven Routing: Uses a JSON config to define which paths are cached (/, /static/*) and which bypass the cache (/api). Supports wildcard patterns.
  - In-Memory Caching: Stores full HTTP responses in a thread-safe HashMap. Tested to reduce latency from ~9ms to <1ms on cache hits.
  - Load Balancing: Distributes non-cached traffic across multiple upstreams using round-robin.
  - Memory Safety: Tracks total cache size atomically. When the limit is reached, it stops caching new large items to prevent OOM crashes instead of panicking.
  - Cache-Status Header: Implements Cache-Status (RFC 9211) to show if a response came from the cache or upstream.

## Cache Performance
Tested in localhost (no network overhead) with a simple Express.js server serving HTML.
On heavy SSR apps (like Next.js) performance should be better, while on Go multi-threaded servers the performance gains should be much lower.

### Single-threaded (~6x faster, tested with `$ hey -c 1 -z 60s http://localhost:8080`)
- Direct upstream (no cache): 422 RPS, 2.4ms avg
- EdgeMap proxy (in-memory cache): 2,534 RPS, 0.4ms avg

### Multi-threaded (~30x faster, tested with `$ hey -c 16 -z 60s http://localhost:8080`)
- Direct upstream (no cache): 662 RPS, 24.2ms avg
- EdgeMap proxy (in-memory cache): 20,197 RPS, 1.0ms avg (+1M req/min)

## Configuration

### Run with a config file:
`cargo run config.json`

Example config.json:

```json
{
  "output_port": 8080,
  "upstreams": ["http://localhost:3000", "http://localhost:3001"],
  "sitemap": [
    {"loc": "/", "priority": 1.0},
    {"loc": "/public/*", "priority": 0.5}
  ],
  "max_memory_mb": 50
}
```

### Or use Lite Mode (single upstream):
`cargo run 3000`

This is a **prototype** and not yet to be used in production. Here is what is still missing or being worked on:

  - Weighted Eviction: currently, when memory is full, the proxy skips caching new items. The plan is to implement a priority-based eviction algorithm that removes low-weight entries first.
  - RFC 9111 Compliance: i am currently implementing Cache-Control, ETag, and Vary header logic to fully respect origin server directives.
  - Zero-Copy Streaming: right now, bodies are buffered into memory. Future refactoring will use raw TCP streaming to handle large files without memory overhead.
  - Scaling: upstream servers are currently static in the config. in the future the project will manage containers/processes to increase the number of instances under load, or scale to zero if the use case allows for mainly cache use of the assets.
  - Protocol Handling: Filtering hop-by-hop headers (Connection, Transfer-Encoding) to comply with RFC 7230 proxy standards.
