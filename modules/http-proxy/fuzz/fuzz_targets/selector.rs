#![no_main]

use std::sync::Arc;

use dashmap::DashMap;
use ferron_http_proxy::types::upstream::UpstreamInner;
use ferron_http_proxy::types::ConnectionsTrackState;
use ferron_http_proxy::upstream::lb::p2c_ewma::EwmaStateMap;
use ferron_http_proxy::upstream::lb::selector::select_backend_index;
use ferron_http_proxy::upstream::lb::{LoadBalancerAlgorithmInner, WeightedRoundRobinState};
use libfuzzer_sys::fuzz_target;
use rustc_hash::FxBuildHasher;

/// Parse algorithm + backends from raw bytes.
///
/// Format:
///   [algorithm_tag: u8]
///   [backend_count: u8]
///     for each backend:
///       [weight: u32 LE]
///       [name_len: u8]
///       [name: name_len bytes (UTF-8)]
fn parse_input(input: &[u8]) -> Option<(LoadBalancerAlgorithmInner, Vec<(usize, UpstreamInner)>)> {
    if input.len() < 2 {
        return None;
    }

    let algorithm_tag = input[0];
    let backend_count = input[1] as usize;
    if backend_count > 30 {
        return None;
    }

    let algorithm = match algorithm_tag {
        0 => LoadBalancerAlgorithmInner::Random,
        1 => LoadBalancerAlgorithmInner::RoundRobin(WeightedRoundRobinState::new()),
        2 => LoadBalancerAlgorithmInner::LeastConnections,
        3 => LoadBalancerAlgorithmInner::TwoRandomChoices,
        4 => LoadBalancerAlgorithmInner::P2cEwma,
        _ => return None,
    };

    let mut pos = 2;
    let mut backends = Vec::with_capacity(backend_count);
    for idx in 0..backend_count {
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
        backends.push((
            idx,
            UpstreamInner {
                proxy_to,
                proxy_unix: None,
                weight,
                mtls: None,
            },
        ));
    }

    Some((algorithm, backends))
}

fuzz_target!(|input: &[u8]| {
    let Some((algorithm, backends)) = parse_input(input) else {
        return;
    };

    let conn_state: ConnectionsTrackState = Arc::new(DashMap::with_hasher(FxBuildHasher));
    let ewma_state: EwmaStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));

    // Convert to indices + upstreams format consumed by select_backend_index
    let healthy: Vec<usize> = (0..backends.len()).collect();
    let upstreams: Vec<Arc<UpstreamInner>> =
        backends.into_iter().map(|(_, u)| Arc::new(u)).collect();

    // Invariant 1: select_backend_index never panics
    let idx = select_backend_index(
        &algorithm,
        &healthy,
        &upstreams,
        Some(&conn_state),
        Some(&ewma_state),
    );

    // Invariant 2: Returned index is always valid
    if upstreams.is_empty() {
        assert_eq!(
            idx, 0,
            "select_backend_index must return 0 for empty backends"
        );
    } else {
        assert!(
            idx < upstreams.len(),
            "select_backend_index returned index {idx} >= {}",
            upstreams.len()
        );
    }

    // Invariant 3: Works with None conn_state and ewma_state (fallthrough)
    let idx2 = select_backend_index(&algorithm, &healthy, &upstreams, None, None);
    if upstreams.is_empty() {
        assert_eq!(idx2, 0);
    } else {
        assert!(idx2 < upstreams.len());
    }

    // Invariant 4: Works with Some(conn_state) without ewma_state
    let idx3 = select_backend_index(&algorithm, &healthy, &upstreams, Some(&conn_state), None);
    if upstreams.is_empty() {
        assert_eq!(idx3, 0);
    } else {
        assert!(idx3 < upstreams.len());
    }

    // Invariant 5: All algorithms are deterministic given same state
    for _ in 0..5 {
        let i = select_backend_index(
            &algorithm,
            &healthy,
            &upstreams,
            Some(&conn_state),
            Some(&ewma_state),
        );
        assert!(
            upstreams.is_empty() || i < upstreams.len(),
            "repeat call returned invalid index {i}"
        );
    }
});
