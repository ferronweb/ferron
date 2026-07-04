#![no_main]

use std::sync::Arc;

use dashmap::DashMap;
use ferron_http_proxy::types::upstream::UpstreamInner;
use ferron_http_proxy::types::ConnectionsTrackState;
use ferron_http_proxy::upstream::lb::p2c_ewma::{
    compute_score, get_decayed_ewma, is_warming_up, update_ewma, EwmaStateMap, P2cEwmaParams,
};
use ferron_http_proxy::upstream::lb::selector::select_backend_index;
use ferron_http_proxy::upstream::lb::{
    ConsistentHashRing, LoadBalancerAlgorithmInner, WeightedRoundRobinState,
};
use libfuzzer_sys::fuzz_target;
use rustc_hash::FxBuildHasher;

/// Parse backends and keys from raw bytes.
///
/// Format:
///   [selector: u8] (0=consistent_hash, 1=weighted_rr, 2=p2c_ewma, 3=selector)
///   [backend_count: u8]
///     for each backend:
///       [weight: u32 LE]
///       [name_len: u8]
///       [name: name_len bytes (UTF-8)]
///   [extra: remaining bytes for target-specific input]
fn parse_input(input: &[u8]) -> Option<(u8, Vec<Arc<UpstreamInner>>, &[u8])> {
    if input.len() < 2 {
        return None;
    }

    let selector = input[0];
    let backend_count = input[1] as usize;
    if !(1..=30).contains(&backend_count) {
        return None;
    }

    let mut pos = 2;
    let mut backends = Vec::with_capacity(backend_count);
    for _ in 0..backend_count {
        if pos + 4 > input.len() {
            return None;
        }
        let weight =
            u32::from_le_bytes([input[pos], input[pos + 1], input[pos + 2], input[pos + 3]]);
        pos += 4;

        if pos + 1 > input.len() {
            return None;
        }
        let name_len = input[pos] as usize;
        pos += 1;

        if pos + name_len > input.len() || name_len > 128 {
            return None;
        }
        let name_bytes = &input[pos..pos + name_len];
        pos += name_len;

        let proxy_to = String::from_utf8(name_bytes.to_vec()).ok()?;
        backends.push(Arc::new(UpstreamInner {
            proxy_to,
            proxy_unix: None,
            weight,
            mtls: None,
            priority: 0,
            connect_to: None,
            connection_timeout: None,
        }));
    }

    Some((selector % 4, backends, &input[pos..]))
}

fuzz_target!(|input: &[u8]| {
    let Some((selector, backends, extra)) = parse_input(input) else {
        return;
    };

    match selector {
        0 => {
            // Consistent hash ring
            let ring = ConsistentHashRing::new(&backends);

            if backends.is_empty() {
                for key in extra.chunks(32) {
                    assert!(
                        ring.get(key, &Default::default()).is_none(),
                        "empty ring must return None"
                    );
                }
                return;
            }

            for key in extra.chunks(32) {
                if let Some(idx) = ring.get(key, &Default::default()) {
                    assert!(
                        idx < backends.len(),
                        "ring.get() returned index {idx} >= {} backends",
                        backends.len()
                    );
                }
            }

            for key in extra.chunks(32) {
                if let (Some(a), Some(b)) = (
                    ring.get(key, &Default::default()),
                    ring.get(key, &Default::default()),
                ) {
                    assert_eq!(a, b, "ring.get() must be deterministic for the same key");
                }
            }

            let rebuild_needed = ring.needs_rebuild(&backends);
            assert!(
                !rebuild_needed,
                "needs_rebuild must be false for same backends"
            );

            if !backends.is_empty() {
                let mut cloned_ring = ring.clone();
                cloned_ring.rebuild(&backends);
                for key in extra.chunks(32) {
                    let orig = ring.get(key, &Default::default());
                    let after = cloned_ring.get(key, &Default::default());
                    assert_eq!(
                        orig, after,
                        "rebuild with same backends must not change get() results"
                    );
                }
            }
        }
        1 => {
            // Weighted round robin
            let state = WeightedRoundRobinState::new();
            let weights: Vec<u32> = backends.iter().map(|b| b.weight.max(1)).collect();

            if weights.is_empty() {
                return;
            }

            for _ in 0..20.min(weights.len().saturating_mul(3)) {
                let idx = state.next(&weights);
                assert!(
                    idx < weights.len(),
                    "next() returned index {idx} >= {}",
                    weights.len()
                );
            }
        }
        2 => {
            // P2C EWMA
            let ewma_state: EwmaStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
            let params = P2cEwmaParams::default();

            for chunk in extra.chunks(12) {
                if chunk.len() < 12 {
                    continue;
                }
                let op = chunk[0] % 3;
                let idx = chunk[1] as usize % backends.len().max(1);
                let upstream = &backends[idx];

                let value = f64::from_le_bytes([
                    chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7], chunk[8], chunk[9],
                ]);

                match op {
                    0 => {
                        update_ewma(&ewma_state, upstream, value, &params);
                        if let Some(data) = ewma_state.get(upstream) {
                            assert!(
                                data.ewma.is_finite(),
                                "EWMA must be finite after update, got: {}",
                                data.ewma
                            );
                        }
                    }
                    1 => {
                        let ewma = get_decayed_ewma(&ewma_state, upstream, &params);
                        assert!(
                            ewma.is_finite(),
                            "get_decayed_ewma must return finite value, got: {}",
                            ewma
                        );
                        assert!(
                            ewma >= 0.0,
                            "get_decayed_ewma must return non-negative value, got: {}",
                            ewma
                        );
                    }
                    2 => {
                        let active_conns = value as usize;
                        let some_ewma = get_decayed_ewma(&ewma_state, upstream, &params);
                        let score = compute_score(some_ewma, active_conns, &params);
                        assert!(
                            score.is_finite(),
                            "compute_score must return finite value, got: {}",
                            score
                        );
                        assert!(
                            score >= 0.0,
                            "compute_score must return non-negative value, got: {}",
                            score
                        );
                    }
                    _ => unreachable!(),
                }
            }

            for backend in &backends {
                let _ = is_warming_up(&ewma_state, backend);
            }
        }
        3 => {
            // Selector
            let algorithm_tag = extra.first().copied().unwrap_or(0);
            let algorithm = match algorithm_tag % 5 {
                0 => LoadBalancerAlgorithmInner::Random,
                1 => LoadBalancerAlgorithmInner::RoundRobin(WeightedRoundRobinState::new()),
                2 => LoadBalancerAlgorithmInner::LeastConnections,
                3 => LoadBalancerAlgorithmInner::TwoRandomChoices,
                4 => LoadBalancerAlgorithmInner::P2cEwma,
                _ => unreachable!(),
            };

            let conn_state: ConnectionsTrackState = Arc::new(DashMap::with_hasher(FxBuildHasher));
            let ewma_state: EwmaStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
            let healthy: Vec<usize> = (0..backends.len()).collect();

            let idx = select_backend_index(
                &algorithm,
                &healthy,
                &backends,
                Some(&conn_state),
                Some(&ewma_state),
            )
            .index;

            if backends.is_empty() {
                assert_eq!(
                    idx, 0,
                    "select_backend_index must return 0 for empty backends"
                );
            } else {
                assert!(
                    idx < backends.len(),
                    "select_backend_index returned index {idx} >= {}",
                    backends.len()
                );
            }

            let idx2 = select_backend_index(&algorithm, &healthy, &backends, None, None).index;
            if backends.is_empty() {
                assert_eq!(idx2, 0);
            } else {
                assert!(idx2 < backends.len());
            }
        }
        _ => unreachable!(),
    }
});
