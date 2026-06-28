#![no_main]

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use std::sync::atomic::AtomicUsize;
use std::sync::{atomic::Ordering, Arc};
use std::thread;

use ferron_http_ratelimit::registry::TokenBucketRegistry;

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);

    let threads = match u.arbitrary::<u8>() {
        Ok(t) => ((t as usize) % 32) + 1,
        Err(_) => 2,
    };

    let num_keys = match u.arbitrary::<u8>() {
        Ok(k) => ((k as usize) % 8) + 1,
        Err(_) => 1,
    };

    let num_ops = match u.arbitrary::<u16>() {
        Ok(n) => ((n as usize) % 1000) + 1,
        Err(_) => 100,
    };

    let capacity = match u.arbitrary::<u16>() {
        Ok(c) => ((c as u64) % 20) + 1,
        Err(_) => 5,
    };

    let refill_rate = 0.0_f64;
    let ttl_secs = 60u64;
    let max_buckets = std::cmp::max(10, num_keys * 2);

    let registry = Arc::new(TokenBucketRegistry::new(
        capacity,
        refill_rate,
        ttl_secs,
        max_buckets,
    ));

    let keys: Vec<String> = (0..num_keys).map(|i| format!("key-{}", i)).collect();
    let successes: Vec<Arc<AtomicUsize>> = (0..num_keys)
        .map(|_| Arc::new(AtomicUsize::new(0)))
        .collect();

    let ops_per_thread = std::cmp::max(1, num_ops / threads);

    let mut handles = Vec::with_capacity(threads);
    for thread_id in 0..threads {
        let registry = Arc::clone(&registry);
        let keys = keys.clone();
        let successes = successes.clone();
        let start_op = thread_id * ops_per_thread;
        let end_op = if thread_id == threads - 1 {
            num_ops
        } else {
            std::cmp::min(start_op + ops_per_thread, num_ops)
        };

        handles.push(thread::spawn(move || {
            for op_idx in start_op..end_op {
                let key_idx = op_idx % num_keys;
                let key = &keys[key_idx];
                if let Some(bucket) = registry.get_or_create(key) {
                    if bucket.try_consume(1) {
                        successes[key_idx].fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
        }));
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }

    for (key_idx, key) in keys.iter().enumerate() {
        let total_successes = successes[key_idx].load(Ordering::SeqCst);
        assert!(
            total_successes <= capacity as usize,
            "key '{}': total successes {} exceeds capacity {}",
            key,
            total_successes,
            capacity
        );
    }
});
