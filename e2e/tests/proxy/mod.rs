#[path = "../common/mod.rs"]
mod common;

mod affinity;
mod grpc;
mod lb;
mod priority;
mod proxy_cache;
mod proxy_circuit_breaker_latency;
mod proxy_circuit_breaker_slow_start;
mod proxy_circuit_breaker_status_code;
mod proxy_failover;
mod proxy_header;
mod proxy_redirect;
mod rproxy;
mod srv;
mod strict_dns;
