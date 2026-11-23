#[path = "../common/mod.rs"]
mod common;

mod affinity;
mod circuit_half_open_timeout;
mod circuit_latency;
mod circuit_status_code;
mod dns_resolution;
mod failover;
mod grpc;
mod header;
mod lb;
mod priority;
mod retry_interval;
mod rproxy;
mod same_upstream_retry;
mod srv;
mod strict_dns;
