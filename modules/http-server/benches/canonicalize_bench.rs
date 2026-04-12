use criterion::{criterion_group, criterion_main, Criterion, black_box};

use ferron_http_server::util::canonicalize_url::{canonicalize_path, canonicalize_path_routing};

fn bench_canonicalize_routing(c: &mut Criterion) {
    let mut group = c.benchmark_group("canonicalize_routing");

    let inputs = vec![
        "/",
        "/api/v2/resource",
        "/a/b/c/d/e/f/g",
        "/%41pi",
        "/api%2Fv2",
        "/a/../b/./c//d",
        "/a%2F%2F%2F/b%20c",
    ];

    for input in inputs {
        group.bench_function(input, |b| {
            let s = input.to_string();
            b.iter(|| {
                let _ = canonicalize_path_routing(black_box(s.as_str()));
            })
        });
    }

    group.finish();
}

fn bench_canonicalize_full(c: &mut Criterion) {
    let mut group = c.benchmark_group("canonicalize_full");

    let inputs = vec![
        "/",
        "/api/v2/resource",
        "/a/b/c/d/e/f/g",
        "/%41pi",
        "/api%2Fv2",
        "/a/../b/./c//d",
        "/a%2F%2F%2F/b%20c",
    ];

    for input in inputs {
        group.bench_function(input, |b| {
            let s = input.to_string();
            b.iter(|| {
                let _ = canonicalize_path(black_box(s.as_str()));
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_canonicalize_routing, bench_canonicalize_full);
criterion_main!(benches);
