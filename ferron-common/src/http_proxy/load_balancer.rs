use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::{LoadBalancerAlgorithmInner, ProxyToKey, ProxyToKeyInner, UpstreamInner};
use crate::util::TtlCache;

/// Selects an index for a backend server based on the load balancing algorithm.
async fn select_backend_index(
  load_balancer_algorithm: &LoadBalancerAlgorithmInner,
  backends: &[ProxyToKeyInner],
) -> usize {
  match load_balancer_algorithm {
    LoadBalancerAlgorithmInner::TwoRandomChoices(connection_track) => {
      let random_choice1 = rand::random_range(..backends.len());
      let mut random_choice2 = if backends.len() > 1 {
        rand::random_range(..(backends.len() - 1))
      } else {
        0
      };
      if backends.len() > 1 && random_choice2 >= random_choice1 {
        random_choice2 += 1;
      }
      let backend1 = &backends[random_choice1];
      let backend2 = &backends[random_choice2];
      let connection_track_read = connection_track.read().await;
      let connection_count_option1 = connection_track_read
        .get(&backend1.0)
        .map(|connection_count| Arc::strong_count(connection_count) - 1);
      let connection_count_option2 = connection_track_read
        .get(&backend2.0)
        .map(|connection_count| Arc::strong_count(connection_count) - 1);
      drop(connection_track_read);
      let connection_count1 = if let Some(count) = connection_count_option1 {
        count
      } else {
        connection_track.write().await.insert(backend1.0.clone(), Arc::new(()));
        0
      };
      let connection_count2 = if let Some(count) = connection_count_option2 {
        count
      } else {
        connection_track.write().await.insert(backend2.0.clone(), Arc::new(()));
        0
      };
      if connection_count2 >= connection_count1 {
        random_choice1
      } else {
        random_choice2
      }
    }
    LoadBalancerAlgorithmInner::LeastConnections(connection_track) => {
      let mut min_indexes = Vec::new();
      let mut min_connections = None;
      for (index, (upstream, _, _)) in backends.iter().enumerate() {
        let connection_track_read = connection_track.read().await;
        let connection_count = if let Some(connection_count) = connection_track_read.get(upstream) {
          Arc::strong_count(connection_count) - 1
        } else {
          drop(connection_track_read);
          connection_track.write().await.insert((*upstream).clone(), Arc::new(()));
          0
        };
        if min_connections.is_none_or(|min| connection_count < min) {
          min_indexes = vec![index];
          min_connections = Some(connection_count);
        } else {
          min_indexes.push(index);
        }
      }
      match min_indexes.len() {
        0 => 0,
        1 => min_indexes[0],
        _ => min_indexes[rand::random_range(0..min_indexes.len())],
      }
    }
    LoadBalancerAlgorithmInner::RoundRobin(round_robin_index) => {
      round_robin_index.fetch_add(1, Ordering::Relaxed) % backends.len()
    }
    LoadBalancerAlgorithmInner::Random => rand::random_range(..backends.len()),
  }
}

/// Determines which backend server to proxy the request to.
#[inline]
pub(super) async fn determine_proxy_to(
  proxy_to_vector: &mut Vec<ProxyToKeyInner>,
  failed_backends: &RwLock<TtlCache<UpstreamInner, u64>>,
  enable_health_check: bool,
  health_check_max_fails: u64,
  load_balancer_algorithm: &LoadBalancerAlgorithmInner,
) -> Option<ProxyToKeyInner> {
  let mut proxy_to = None;

  if proxy_to_vector.is_empty() {
    return None;
  } else if proxy_to_vector.len() == 1 {
    let proxy_to_borrowed = proxy_to_vector.remove(0);
    let upstream = proxy_to_borrowed.0;
    let local_limit_index = proxy_to_borrowed.1;
    let keepalive_idle_timeout = proxy_to_borrowed.2;
    proxy_to = Some((upstream, local_limit_index, keepalive_idle_timeout));
  } else if enable_health_check {
    loop {
      if !proxy_to_vector.is_empty() {
        let index = select_backend_index(load_balancer_algorithm, proxy_to_vector).await;
        let proxy_to_borrowed = proxy_to_vector.remove(index);
        let upstream = proxy_to_borrowed.0;
        let local_limit_index = proxy_to_borrowed.1;
        let keepalive_idle_timeout = proxy_to_borrowed.2;
        let failed_backends_read = failed_backends.read().await;
        let failed_backend_fails_option = failed_backends_read.get(&upstream);
        proxy_to = Some((upstream, local_limit_index, keepalive_idle_timeout));
        let failed_backend_fails = if let Some(fails) = failed_backend_fails_option {
          fails
        } else {
          break;
        };
        if failed_backend_fails <= health_check_max_fails {
          break;
        }
      } else {
        break;
      }
    }
  } else if !proxy_to_vector.is_empty() {
    let index = select_backend_index(load_balancer_algorithm, proxy_to_vector).await;
    let proxy_to_borrowed = proxy_to_vector.remove(index);
    let upstream = proxy_to_borrowed.0;
    let local_limit_index = proxy_to_borrowed.1;
    let keepalive_idle_timeout = proxy_to_borrowed.2;
    proxy_to = Some((upstream, local_limit_index, keepalive_idle_timeout));
  }

  proxy_to
}

/// Resolves inner upstreams from a list of upstreams.
pub(super) async fn resolve_upstreams(
  proxy_to: &[ProxyToKey],
  failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
  health_check_max_fails: u64,
) -> Vec<ProxyToKeyInner> {
  let mut upstreams = Vec::new();
  for proxy_to in proxy_to {
    let upstream = proxy_to
      .0
      .resolve(failed_backends.clone(), health_check_max_fails)
      .await;
    for upstream in upstream {
      upstreams.push((upstream, proxy_to.1, proxy_to.2));
    }
  }
  upstreams
}
