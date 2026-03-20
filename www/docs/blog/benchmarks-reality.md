# Why Raw Benchmarks Don't Tell the Full Story

## The Problem with Synthetic Benchmarks

When evaluating programming languages and frameworks, we often see headlines like "X is 10x faster than Y" based on simple benchmarks like hello-world or JSON serialization. These numbers are misleading because they don't reflect real-world workloads.

## What Raw Benchmarks Miss

### 1. Cold Start Times
A language might handle 100k requests per second in a warm state, but if it takes 500ms to start, your actual user experience is terrible.

### 2. Memory Pressure
High-throughput benchmarks ignore what happens when memory is constrained. Does the framework start swapping? Crash? Or gracefully degrade?

### 3. Real Network Conditions
Production traffic has latency spikes, connection timeouts, and packet loss. Synthetic benchmarks run in ideal conditions.

### 4. Complex Interactions
A blog might handle JSON fine, but fail under load when combined with:
- Database connections
- Session management
- Authentication checks
- Rate limiting
- WebSocket connections

## Enter oha: HTTP Load Testing

[oha](https://github.com/hatoo/oha) is a Rust-based HTTP load testing tool that benchmarks against actual endpoints, not micro-benchmarks.

```bash
oha -n 10000 -c 100 https://your-endpoint
```

### Why oha is Better

1. **Realistic负载**: Tests actual HTTP endpoints with real network stacks
2. **End-to-End**: Includes DNS resolution, TLS handshakes, connection pooling
3. **Global Testing**: Can test from multiple regions to simulate international users
4. **Detailed Metrics**: Shows latency percentiles (p50, p95, p99), throughput, and errors

## Example: Comparing Frameworks

Instead of running:
```bash
wrk -t12 -c400 -d10s http://localhost/hello
```

Run:
```bash
oha -n 100000 -c 100 -q10 https://your-production-url/api/v1/users
```

This tests:
- Your actual routing
- Middleware stack
- Database queries
- Response serialization
- Connection pool limits

## What Matters for Production

When we evaluate Soli, we care about:

| Metric | Why It Matters |
|--------|---------------|
| **p99 Latency** | Affects worst-case user experience |
| **Throughput under load** | Can it handle traffic spikes? |
| **Memory stability** | Does it leak over time? |
| **Cold start** | Serverless deployment viability |
| **Error rate** | Under partial failure conditions |

## Conclusion

The next time you see "X is faster than Y", ask:
- What workload was tested?
- Was it cold or warm?
- Did it include middleware?
- Was it tested in production-like conditions?

Raw benchmarks are useful for micro-optimizations, but real performance understanding comes from end-to-end testing with tools like oha.