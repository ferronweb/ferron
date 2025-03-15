---
title: "Introducing FerrBench: a modern HTTP benchmarking tool"
description: We are excited to introduce FerrBench, a modern HTTP benchmarking tool designed to provide realistic performance metrics for modern web frameworks and servers.
date: 2025-03-15 16:17:00
cover: /img/covers/introducing-ferrbench-a-modern-http-benchmarking-tool.png
---

We are excited to announce FerrBench, a modern HTTP benchmarking tool designed to provide realistic performance metrics for modern web frameworks and servers. Based on [rewrk](https://github.com/lnx-search/rewrk) and powered by [hyper's client API](https://github.com/hyperium/hyper), FerrBench offers both HTTP/1 and HTTP/2 benchmarking without the outdated biases of HTTP/1 pipelining.

## Why FerrBench?
Benchmarking tools like [wrk](https://github.com/wg/wrk) have been widely used in performance testing, but they come with limitations—most notably, their reliance on HTTP/1 pipelining, which can unfairly favor certain frameworks. Modern web applications primarily operate with pipelining disabled, making those benchmarks less relevant in real-world scenarios.

FerrBench addresses these issues by:
- **Supporting HTTP/1 and HTTP/2**, providing more accurate and relevant performance measurements.
- **Disabling HTTP/1 pipelining** to prevent misleading results.
- **Being cross-platform**, allowing developers to run benchmarks seamlessly on various operating systems.

## Key features
- **Multi-threaded benchmarking** - run benchmarks with multiple threads (`-t`) and concurrent connections (`-c`).
- **Percentile-based latency reporting** - use `--pct` to gain insights into 99th and 95th percentile latencies.
- **Custom request options** - specify HTTP methods (`-m`), headers (`-H`), and request bodies (`-b`).
- **JSON output support** - use `--json` for machine-readable results.
- **Repeatable benchmarks** - the `-r` flag allows multiple rounds of benchmarking for deeper analysis.

## Example benchmark
Running a test with 60 concurrent connections, 12 threads, and a 5-second duration:
```sh
ferrbench -h http://127.0.0.1:5000 -t 12 -c 60 -d 5s
```
**Output:**
```
Benchmarking 60 connections @ http://127.0.0.1:5000 for 5 seconds
  Latencies:
    Avg      Stdev    Min      Max    
    3.27ms   0.40ms   1.95ms   9.39ms
  Requests:
    Total:  91281  Req/Sec: 18227.81
  Transfer:
    Total: 1.13 MB Transfer Rate: 231.41 KB/Sec
```

## Getting started
### Install via Cargo
```
cargo install ferrbench --git https://github.com/ferronweb/ferrbench.git
```
### Build from source
1. Clone the repository:
```
git clone https://github.com/ferronweb/ferrbench.git && cd ferrbench
```
2. Build the binary:
```
cargo build --release
```
3. Run the benchmarker:
```
target/release/ferrbench -h http://127.0.0.1:5000 -c 256 -t 12 -d 15s --http2 --pct
```

## FerrBench & Ferron: an unified vision
FerrBench is maintained by the same team behind [Ferron](https://github.com/ferronweb/ferron), a fast, memory-safe web server written in Rust. Our goal is to empower developers with both high-performance web infrastructure and accurate benchmarking tools to optimize their applications.

We encourage developers to test FerrBench, contribute feedback, and help us refine benchmarking methodologies for the modern web era.

Get started with FerrBench today and ensure your performance tests reflect real-world conditions!

You can check out the [FerrBench's GitHub repository](https://github.com/ferronweb/ferrbench)!
