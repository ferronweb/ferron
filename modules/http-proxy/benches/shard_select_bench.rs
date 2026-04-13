use criterion::{criterion_group, criterion_main, Criterion};
use std::net::IpAddr;

use ferron_http_proxy::connections::ConnectionManager;
use ferron_http_proxy::upstream::UpstreamInner;

fn bench_select_pool(c: &mut Criterion) {
    let cm = ConnectionManager::with_global_limit_and_shards(100, 8);
    let upstream = UpstreamInner {
        proxy_to: "http://127.0.0.1:3000".to_string(),
        proxy_unix: None,
    };
    let client_ip: Option<IpAddr> = None;

    c.bench_function("select_pool_shard_hash", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                let _ = cm.select_pool(&upstream, client_ip);
            }
        });
    });
}

criterion_group!(benches, bench_select_pool);
criterion_main!(benches);
