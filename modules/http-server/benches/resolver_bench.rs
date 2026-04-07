use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use ferron_http_server::config::prepare::{
    PreparedHostConfigurationBlock, PreparedHostConfigurationMatch,
    PreparedHostConfigurationMatcher,
};
use ferron_http_server::config::resolver::{
    ResolvedLocationPath, Stage2RadixResolver, ThreeStageResolver,
};
use ferron_http_server::config::HostConfigs;
use http_body_util::BodyExt;
use std::hint::black_box;
use std::sync::Arc;

/// Create a test configuration block
fn create_test_block() -> PreparedHostConfigurationBlock {
    PreparedHostConfigurationBlock {
        directives: Arc::new(std::collections::HashMap::default()),
        matches: Vec::new(),
        error_config: Vec::new(),
    }
}

/// Setup a simple resolver with a single hostname
fn setup_single_host() -> Stage2RadixResolver {
    let mut resolver = Stage2RadixResolver::new();
    let config = Arc::new(create_test_block());
    resolver.insert_host(vec!["com", "example"], config, 10);
    resolver
}

/// Setup a resolver with multiple hostnames sharing a TLD
fn setup_shared_tld() -> Stage2RadixResolver {
    let mut resolver = Stage2RadixResolver::new();
    for sub in &["www", "api", "admin", "blog", "shop"] {
        let config = Arc::new(create_test_block());
        resolver.insert_host(vec!["com", "example", sub], config, 10);
    }
    resolver
}

/// Setup a resolver with deep hostname chains
fn setup_deep_chain() -> Stage2RadixResolver {
    let mut resolver = Stage2RadixResolver::new();
    let config = Arc::new(create_test_block());
    resolver.insert_host(
        vec!["com", "example", "dept", "team", "project"],
        config,
        10,
    );
    resolver
}

/// Setup a resolver with wildcards
fn setup_wildcards() -> Stage2RadixResolver {
    let mut resolver = Stage2RadixResolver::new();

    // Exact matches
    let config = Arc::new(create_test_block());
    resolver.insert_host(vec!["com", "example"], config, 10);

    // Wildcard matches
    let wildcard_config = Arc::new(create_test_block());
    resolver.insert_host_wildcard(vec!["com", "example"], wildcard_config, 5);

    // Multiple wildcards at different levels
    let wildcard_config2 = Arc::new(create_test_block());
    resolver.insert_host_wildcard(vec!["com"], wildcard_config2, 1);

    resolver
}

/// Setup a resolver with many hostnames (simulating real-world config)
fn setup_many_hosts() -> Stage2RadixResolver {
    let mut resolver = Stage2RadixResolver::new();

    let domains = [
        ("com", "example", "www"),
        ("com", "example", "api"),
        ("com", "example", "admin"),
        ("com", "example", "blog"),
        ("com", "example", "shop"),
        ("com", "test", "www"),
        ("com", "test", "api"),
        ("org", "mysite", "www"),
        ("org", "mysite", "api"),
        ("net", "service", "app"),
        ("io", "app", "dashboard"),
        ("io", "app", "api"),
        ("io", "app", "admin"),
    ];

    for (tld, domain, sub) in domains.iter() {
        let config = Arc::new(create_test_block());
        resolver.insert_host(vec![tld, domain, sub], config, 10);
    }

    // Add some wildcards
    let wildcard_config = Arc::new(create_test_block());
    resolver.insert_host_wildcard(vec!["com", "example"], wildcard_config, 5);

    resolver
}

fn bench_stage2_hostname_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("stage2_hostname_resolution");

    // Single hostname - exact match
    let single_resolver = setup_single_host();
    group.bench_function("single_host_exact", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs = single_resolver.resolve_hostname(black_box("example.com"), &mut path);
            black_box(configs);
        })
    });

    // Single hostname - miss
    group.bench_function("single_host_miss", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs = single_resolver.resolve_hostname(black_box("other.com"), &mut path);
            black_box(configs);
        })
    });

    // Shared TLD - exact match
    let shared_resolver = setup_shared_tld();
    group.bench_function("shared_tld_exact", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs = shared_resolver.resolve_hostname(black_box("api.example.com"), &mut path);
            black_box(configs);
        })
    });

    // Shared TLD - wildcard match
    group.bench_function("shared_tld_wildcard", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs =
                shared_resolver.resolve_hostname(black_box("random.example.com"), &mut path);
            black_box(configs);
        })
    });

    // Deep chain - exact match
    let deep_resolver = setup_deep_chain();
    group.bench_function("deep_chain_exact", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs = deep_resolver
                .resolve_hostname(black_box("project.team.dept.example.com"), &mut path);
            black_box(configs);
        })
    });

    // Many hosts - exact match
    let many_resolver = setup_many_hosts();
    group.bench_function("many_hosts_exact", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs = many_resolver.resolve_hostname(black_box("api.example.com"), &mut path);
            black_box(configs);
        })
    });

    // Many hosts - miss
    group.bench_function("many_hosts_miss", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs =
                many_resolver.resolve_hostname(black_box("unknown.domain.com"), &mut path);
            black_box(configs);
        })
    });

    group.finish();
}

fn bench_stage2_wildcard_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("stage2_wildcard_resolution");

    let resolver = setup_wildcards();

    // Exact match (highest priority)
    group.bench_function("wildcard_exact_match", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs = resolver.resolve_hostname(black_box("example.com"), &mut path);
            black_box(configs);
        })
    });

    // Single-level wildcard match
    group.bench_function("wildcard_single_level", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs = resolver.resolve_hostname(black_box("www.example.com"), &mut path);
            black_box(configs);
        })
    });

    // Multi-level wildcard match
    group.bench_function("wildcard_multi_level", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs =
                resolver.resolve_hostname(black_box("deep.nested.example.com"), &mut path);
            black_box(configs);
        })
    });

    // Root wildcard match
    group.bench_function("wildcard_root_level", |b| {
        b.iter(|| {
            let mut path = ResolvedLocationPath::new();
            let configs = resolver.resolve_hostname(black_box("anything.com"), &mut path);
            black_box(configs);
        })
    });

    group.finish();
}

fn bench_stage2_full_resolution(c: &mut Criterion) {
    use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};

    let mut group = c.benchmark_group("stage2_full_resolution");

    // Setup resolver with hostname config
    let mut resolver = Stage2RadixResolver::new();
    let mut host_directives = std::collections::HashMap::default();
    host_directives.insert(
        "host_level".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String(
                "host_value".to_string(),
                None,
            )],
            children: None,
            span: None,
        }],
    );
    let host_config = Arc::new(PreparedHostConfigurationBlock {
        directives: Arc::new(host_directives),
        matches: Vec::new(),
        error_config: Vec::new(),
    });
    resolver.insert_host(vec!["com", "example"], host_config, 10);

    // Setup base block with location matcher
    let mut base_directives = std::collections::HashMap::default();
    base_directives.insert(
        "base_level".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String(
                "base_value".to_string(),
                None,
            )],
            children: None,
            span: None,
        }],
    );
    let mut base_block = PreparedHostConfigurationBlock {
        directives: Arc::new(base_directives),
        matches: Vec::new(),
        error_config: Vec::new(),
    };

    // Add location matcher
    let mut location_config = std::collections::HashMap::default();
    location_config.insert(
        "location_directive".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String(
                "location_value".to_string(),
                None,
            )],
            children: None,
            span: None,
        }],
    );
    base_block.matches.push(PreparedHostConfigurationMatch {
        matcher: PreparedHostConfigurationMatcher::Location("/api".to_string()),
        config: Arc::new(PreparedHostConfigurationBlock {
            directives: Arc::new(location_config),
            matches: Vec::new(),
            error_config: Vec::new(),
        }),
    });

    let variables = (
        http::Request::new(
            http_body_util::Empty::new()
                .map_err(|e| match e {})
                .boxed_unsync(),
        ),
        std::collections::HashMap::default(),
    );

    let base_block_arc = Arc::new(base_block);

    // Full resolution with hostname + path
    group.bench_function("full_resolution_api", |b| {
        b.iter(|| {
            let (config, path) = resolver.resolve(
                Some(black_box("example.com")),
                black_box("/api/users"),
                base_block_arc.clone(),
                &variables,
                None,
            );
            black_box(config);
            black_box(path);
        })
    });

    // Full resolution with non-matching path
    group.bench_function("full_resolution_static", |b| {
        b.iter(|| {
            let (config, path) = resolver.resolve(
                Some(black_box("example.com")),
                black_box("/static/file.css"),
                base_block_arc.clone(),
                &variables,
                None,
            );
            black_box(config);
            black_box(path);
        })
    });

    group.finish();
}

fn bench_three_stage_resolver(c: &mut Criterion) {
    let mut group = c.benchmark_group("three_stage_resolver");

    let mut resolver = ThreeStageResolver::new();

    // Setup Stage 1 - IP resolver
    let mut hosts = HostConfigs::default();
    let host_block = create_test_block();
    hosts.insert(Some("example.com".to_string()), Arc::new(host_block));
    resolver
        .stage1()
        .register_ip("192.168.1.1".parse().unwrap(), hosts);

    // Setup Stage 2 - Hostname resolver
    let host_config = Arc::new(create_test_block());
    resolver
        .stage2()
        .insert_host(vec!["com", "example"], host_config, 10);

    // Setup Stage 3 - Error resolver
    let error_config = Arc::new(create_test_block());
    resolver.stage3().register_error(404, error_config);

    let variables = (
        http::Request::new(
            http_body_util::Empty::new()
                .map_err(|e| match e {})
                .boxed_unsync(),
        ),
        std::collections::HashMap::default(),
    );
    let test_ip: std::net::IpAddr = "192.168.1.1".parse().unwrap();

    // Full three-stage resolution
    group.bench_function("three_stage_full", |b| {
        b.iter(|| {
            let result = resolver.resolve(
                black_box(test_ip),
                black_box("example.com"),
                black_box("/api"),
                &variables,
            );
            black_box(result);
        })
    });

    // Stage 1 only
    group.bench_function("three_stage_ip_only", |b| {
        b.iter(|| {
            let result = resolver.resolve_stage1_layered(black_box(test_ip), None);
            black_box(result);
        })
    });

    group.finish();
}

fn bench_tree_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_operations");

    // Benchmark insert_host with compression
    group.bench_function("insert_host_single", |b| {
        b.iter(|| {
            let mut resolver = Stage2RadixResolver::new();
            let config = Arc::new(create_test_block());
            resolver.insert_host(black_box(vec!["com", "example", "www"]), config, 10);
            black_box(resolver);
        })
    });

    // Benchmark insert_host with deep chain
    group.bench_function("insert_host_deep_chain", |b| {
        b.iter(|| {
            let mut resolver = Stage2RadixResolver::new();
            let config = Arc::new(create_test_block());
            resolver.insert_host(
                black_box(vec!["com", "example", "dept", "team", "project", "app"]),
                config,
                10,
            );
            black_box(resolver);
        })
    });

    // Benchmark insert_host_wildcard
    group.bench_function("insert_host_wildcard", |b| {
        b.iter(|| {
            let mut resolver = Stage2RadixResolver::new();
            let config = Arc::new(create_test_block());
            resolver.insert_host_wildcard(black_box(vec!["com", "example"]), config, 5);
            black_box(resolver);
        })
    });

    // Benchmark multiple inserts (builds tree incrementally)
    group.bench_with_input(
        BenchmarkId::new("insert_multiple", "10_hosts"),
        &10,
        |b, &n| {
            b.iter(|| {
                let mut resolver = Stage2RadixResolver::new();
                for i in 0..n {
                    let config = Arc::new(create_test_block());
                    resolver.insert_host(vec!["com", &format!("domain{}", i)], config, 10);
                }
                black_box(resolver);
            })
        },
    );

    group.finish();
}

criterion_group!(
    benches,
    bench_stage2_hostname_resolution,
    bench_stage2_wildcard_resolution,
    bench_stage2_full_resolution,
    bench_three_stage_resolver,
    bench_tree_operations,
);

criterion_main!(benches);
