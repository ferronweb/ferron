use criterion::{criterion_group, criterion_main, Criterion};
use std::fs;
use std::path::PathBuf;
use std::hint::black_box;
use vibeio::blocking::DefaultBlockingThreadPool;

fn bench_resolve_file_pipeline(c: &mut Criterion) {
    // Create a unique temporary directory for the bench
    let tmp_base = std::env::temp_dir().join(format!(
        "ferron_bench_resolve_file_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&tmp_base);
    let _ = fs::write(tmp_base.join("index.html"), b"hello world");
    let assets = tmp_base.join("static");
    let _ = fs::create_dir_all(&assets);
    let _ = fs::write(assets.join("file.js"), b"console.log('ok');");

    // Build a small vibeio runtime to run async file resolution
    let rt = vibeio::RuntimeBuilder::new()
        .enable_timer(true)
        .blocking_pool(Box::new(DefaultBlockingThreadPool::with_max_threads(2)))
        .build()
        .expect("failed to create vibeio runtime");

    let root_path = tmp_base.clone();

    c.bench_function("resolve_index_file", |b| {
        b.iter(|| {
            rt.block_on(async {
                let res = ferron_http_server::bench_resolve_http_file_target(&root_path, "/index.html", None).await;
                black_box(res).ok();
            });
        })
    });

    c.bench_function("resolve_nested_file", |b| {
        b.iter(|| {
            rt.block_on(async {
                let res = ferron_http_server::bench_resolve_http_file_target(&root_path, "/static/file.js", None).await;
                black_box(res).ok();
            });
        })
    });
}

criterion_group!(benches, bench_resolve_file_pipeline);
criterion_main!(benches);
