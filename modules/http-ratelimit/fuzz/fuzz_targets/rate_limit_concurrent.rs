#![no_main]

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use std::sync::{atomic::Ordering, Arc};
use std::sync::atomic::AtomicUsize;
use std::thread;

use ferron_http_ratelimit::registry::TokenBucketRegistry;

fuzz_target!(|data: &[u8]| {
    // Decode structured parameters from input bytes using `arbitrary::Unstructured`.
    let mut u = Unstructured::new(data);

    let threads = match u.arbitrary::<u8>() {
        Ok(t) => ((t as usize) % 32) + 1, // 1..32
        Err(_) => 2,
    };

    let num_keys = match u.arbitrary::<u8>() {
        Ok(k) => ((k as usize) % 8) + 1, // 1..8
        Err(_) => 1,
    };

    let num_ops = match u.arbitrary::<u16>() {
        Ok(n) => ((n as usize) % 1000) + 1, // 1..1000
        Err(_) => 100,
    };

    // Choose a small capacity and deterministic zero refill rate to make the
    // concurrency invariant easy to check: total successes per key must not
    // exceed capacity even under heavy concurrent access.
    let capacity = match u.arbitrary::<u16>() {
        Ok(c) => ((c as u64) % 20) + 1, // 1..20
        Err(_) => 5,
    };

    let refill_rate = 0.0_f64; // deterministic: no automatic refills
    let ttl_secs = 60u64;
    let max_buckets = std::cmp::max(10, num_keys * 2);

    let registry = Arc::new(TokenBucketRegistry::new(capacity, refill_rate, ttl_secs, max_buckets));

    // Prepare keys and a shared counter for successful consumes per key.
    let keys: Vec<String> = (0..num_keys).map(|i| format!("key-{}", i)).collect();
    let successes: Vec<Arc<AtomicUsize>> = (0..num_keys).map(|_| Arc::new(AtomicUsize::new(0))).collect();

    // Precompute operations (each op is a key index). If input runs out, defaults are used.
    let mut ops = Vec::with_capacity(num_ops);
    for _ in 0..num_ops {
        let key_idx = match u.arbitrary::<u8>() {
            Ok(k) => (k as usize) % num_keys,
            Err(_) => 0,
        };
        ops.push(key_idx);
    }

    // Spawn threads and distribute operations among them.
    let mut handles = Vec::with_capacity(threads);

    for t_id in 0..threads {
        let registry = registry.clone();
        let keys = keys.clone();
        let ops = ops.clone();
        let successes_cloned: Vec<_> = successes.iter().map(|a| a.clone()).collect();

        handles.push(thread::spawn(move || {
            for (i, &key_idx) in ops.iter().enumerate() {
                if i % threads != t_id {
                    continue;
                }

                let key = &keys[key_idx];
                if let Some(bucket) = registry.get_or_create(key) {
                    if bucket.try_consume(1) {
                        successes_cloned[key_idx].fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }));
    }

    for h in handles {
        let _ = h.join();
    }

    // Invariant: with zero refill rate, total successful consumes per key must
    // never exceed the configured capacity. If this fails, it's a concurrency
    // integrity bug (e.g., duplicate buckets created for the same key).
    for s in &successes {
        assert!(s.load(Ordering::Relaxed) <= capacity as usize);
    }
});
