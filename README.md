EdgeMap is a reverse proxy I built in Rust to explore high-performance edge caching and load balancing. The goal was to solve the single-thread bottleneck of Node.js applications by offloading static traffic to RAM while keeping the backend alive only for dynamic requests.

It acts as a smart layer between the client and a single/cluster of upstream servers, using a sitemap configuration to decide what gets cached.

 ### Key Features

  - Async Reverse Proxy: Built on tokio and axum, handling concurrent requests without blocking.
  - Sitemap-Driven Routing: Uses a JSON config to define which paths are cached (/, /static/*) and which bypass the cache (/api). Supports wildcard patterns.
  - In-Memory Caching: Stores full HTTP responses in a thread-safe HashMap. Tested to reduce latency from ~9ms to <1ms on cache hits.
  - Load Balancing: Distributes non-cached traffic across multiple upstreams using round-robin.
  - Memory Safety: Tracks total cache size atomically. When the limit is reached, it stops caching new large items to prevent OOM crashes instead of panicking.
  - Cache-Status Header: Implements Cache-Status (RFC 9211) to show if a response came from the cache or upstream.
  - Weighted Eviction: A priority-based eviction algorithm that removes low-weight entries first, the configuration for it currently is in `.json`, with `sitemap.xml` support planned in the future.

## Cache Performance
Tested in localhost (no network overhead) with a simple Express.js server serving HTML.
On heavy SSR apps (like Next.js) performance should be better, while on Go multi-threaded servers the performance gains should be much lower.

### Single-threaded (~6x faster, tested with `$ hey -c 1 -z 60s http://localhost:8080`)
- Direct upstream (no cache): 422 RPS, 2.4ms avg
- EdgeMap proxy (in-memory cache): 2,534 RPS, 0.4ms avg

### Multi-threaded (~30x faster, tested with `$ hey -c 16 -z 60s http://localhost:8080`)
- Direct upstream (no cache): 662 RPS, 24.2ms avg
- EdgeMap proxy (in-memory cache): 20,197 RPS, 1.0ms avg (+1M req/min)

## Round-Robin, OOM Prevention and Weighted Eviction in Action
On this example the config was defined as:
- `/media/videos/*` is not set in the sitemap
- `/media/logos/*` is set as a medium priority
- `/media/cases/*` is set as the highest priority

Results:
```log
DEBUG::<Proxy> - Fetching /media/images/logos/logo_header.png from upstream http://localhost:3001 and storing in cache
DEBUG::<Proxy> - Bypassing /media/videos/hero_background.mp4 to upstream http://localhost:3000
DEBUG::<Proxy> - Fetching /media/images/cases/client_a.jpg from upstream http://localhost:3001 and storing in cache
DEBUG::<Proxy> - Fetching /media/images/cases/client_b.jpg from upstream http://localhost:3000 and storing in cache
DEBUG::<Proxy> - Fetching /media/images/cases/client_c.jpg from upstream http://localhost:3001 and storing in cache
DEBUG::<Cache> - Evicted /media/images/logos/logo_footer.png - Freeing 2116 bytes
DEBUG::<Cache> - Evicted /media/images/logos/logo_hero.png - Freeing 4743 bytes
DEBUG::<Cache> - Evicted /media/images/logos/logo_header.png - Freeing 3387 bytes
DEBUG::<Cache> - Successfully evicted enough memory for /media/images/cases/client_d.jpg, storing in cache
DEBUG::<Proxy> - Loaded 54885 bytes from cache memory
DEBUG::<Proxy> - Loaded 83593 bytes from cache memory
DEBUG::<Cache> - Cache full (1MB). Skipping cache for: /media/images/logo/logo_dark.jpg
```

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

  - RFC 9111 Compliance: i am currently implementing Cache-Control, ETag, and Vary header logic to fully respect origin server directives.
  - Support for sitemap.xml: during initial development im using a quick and easy serde for the config.json, in the future the proxy will have a third mode (a mix of fully configurated and lite mode), which will use the standard sitemap.xml which most websites already have as valid configuration.
  - Zero-Copy Streaming: right now, bodies are buffered into memory. Future refactoring will use raw TCP streaming to handle large files without memory overhead.
  - Scaling: upstream servers are currently static in the config. in the future the project will manage containers/processes to increase the number of instances under load, or scale to zero if the use case allows for mainly cache use of the assets.
  - Protocol Handling: Filtering hop-by-hop headers (Connection, Transfer-Encoding) to comply with RFC 7230 proxy standards.
  - TLS Support: currently, attempting to use HTTPS as upstreams causes undefined behavior, im planning to add this after the main features using `rustls`
