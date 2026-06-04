#![no_main]

use std::sync::Arc;

use dashmap::DashMap;
use ferron_http_proxy::types::upstream::UpstreamInner;
use ferron_http_proxy::upstream::lb::p2c_ewma::{
    compute_score, get_decayed_ewma, is_warming_up, update_ewma, EwmaStateMap, P2cEwmaParams,
};
use libfuzzer_sys::fuzz_target;
use rustc_hash::FxBuildHasher;

/// Parse P2C+EWMA operations from raw bytes.
///
/// Format:
///   [backend_count: u8]
///     for each backend:
///       [name_len: u8]
///       [name: name_len bytes (UTF-8)]
///   [op_count: u8]
///     for each op:
///       [op_tag: u8]  (0 = update, 1 = get_decayed, 2 = compute_score)
///       [backend_idx: u8]
///       [latency_or_connections: f64 LE or u32 LE depending on op]
fn parse_input(input: &[u8]) -> Option<(Vec<Arc<UpstreamInner>>, Vec<u8>, Vec<f64>)> {
    if input.is_empty() {
        return None;
    }

    let mut pos = 0;

    let backend_count = input[pos] as usize;
    pos += 1;

    if backend_count > 20 {
        return None;
    }

    let mut backends = Vec::with_capacity(backend_count);
    for _ in 0..backend_count {
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
            weight: 1,
        }));
    }

    if pos + 1 > input.len() {
        return None;
    }
    let op_count = input[pos] as usize;
    pos += 1;

    if op_count > 50 {
        return None;
    }

    let mut ops = Vec::with_capacity(op_count);
    let mut values = Vec::with_capacity(op_count);
    for _ in 0..op_count {
        if pos + 2 > input.len() {
            return None;
        }
        let op_tag = input[pos];
        let backend_idx = input[pos + 1] as usize;
        pos += 2;

        if op_tag > 2 || backend_idx >= backends.len() {
            return None;
        }

        let value = if op_tag == 0 || op_tag == 1 {
            // latency_secs as f64
            if pos + 8 > input.len() {
                return None;
            }
            let val = f64::from_le_bytes([
                input[pos],
                input[pos + 1],
                input[pos + 2],
                input[pos + 3],
                input[pos + 4],
                input[pos + 5],
                input[pos + 6],
                input[pos + 7],
            ]);
            pos += 8;
            val
        } else {
            // active_connections as f64
            if pos + 4 > input.len() {
                return None;
            }
            let conns =
                u32::from_le_bytes([input[pos], input[pos + 1], input[pos + 2], input[pos + 3]]);
            pos += 4;
            conns as f64
        };

        ops.push(op_tag);
        values.push(value);
    }

    Some((backends, ops, values))
}

fuzz_target!(|input: &[u8]| {
    let Some((backends, ops, values)) = parse_input(input) else {
        return;
    };

    let ewma_state: EwmaStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
    let params = P2cEwmaParams::default();

    for (&op, &value) in ops.iter().zip(values.iter()) {
        let idx = (op as usize) % backends.len().max(1);
        let upstream = &backends[idx];

        match op {
            0 => {
                // update_ewma — must never panic or corrupt state
                update_ewma(&ewma_state, upstream, value, &params);

                // Invariant: EWMA must always be finite after update
                if let Some(data) = ewma_state.get(upstream) {
                    assert!(
                        data.ewma.is_finite(),
                        "EWMA must be finite after update, got: {}",
                        data.ewma
                    );
                }
            }
            1 => {
                // get_decayed_ewma — must always return finite
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
                // compute_score — must always return finite non-negative
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

    // Invariant: is_warming_up must not panic for any backend
    for backend in &backends {
        let _ = is_warming_up(&ewma_state, backend);
    }
});
