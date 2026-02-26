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
      // *1 - random choice #1
      // *2 - random choice #2
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
          // Less connections than minimum
          min_indexes = vec![index];
          min_connections = Some(connection_count);
        } else if min_connections == Some(connection_count) {
          // Same amount of connections
          min_indexes.push(index);
        }
      }
      match min_indexes.len() {
        0 => 0, // Possible edge case
        1 => min_indexes[0],
        _ => min_indexes[rand::random_range(0..min_indexes.len())],
      }
    }
    LoadBalancerAlgorithmInner::RoundRobin(round_robin_index) => {
      // Add to round robin index, then modulo the length of backends to prevent overflow
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

#[cfg(test)]
mod tests {
  use std::collections::HashMap;
  use std::future::Future;
  use std::sync::atomic::AtomicUsize;
  use std::time::Duration;

  use super::*;

  fn run_async<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .expect("runtime should be created")
      .block_on(future)
  }

  fn upstream(proxy_to: &str) -> UpstreamInner {
    UpstreamInner {
      proxy_to: proxy_to.to_string(),
      proxy_unix: None,
    }
  }

  #[test]
  fn round_robin_cycles_through_backends() {
    run_async(async {
      let backends = vec![
        (upstream("http://backend-1"), None, None),
        (upstream("http://backend-2"), None, None),
        (upstream("http://backend-3"), None, None),
      ];
      let algorithm = LoadBalancerAlgorithmInner::RoundRobin(Arc::new(AtomicUsize::new(0)));

      assert_eq!(select_backend_index(&algorithm, &backends).await, 0);
      assert_eq!(select_backend_index(&algorithm, &backends).await, 1);
      assert_eq!(select_backend_index(&algorithm, &backends).await, 2);
      assert_eq!(select_backend_index(&algorithm, &backends).await, 0);
    });
  }

  #[test]
  fn least_connections_picks_backend_with_lowest_connection_count() {
    run_async(async {
      let heavily_loaded = upstream("http://backend-1");
      let least_loaded = upstream("http://backend-2");
      let moderately_loaded = upstream("http://backend-3");

      let heavily_loaded_tracker = Arc::new(());
      let _heavy_1 = heavily_loaded_tracker.clone();
      let _heavy_2 = heavily_loaded_tracker.clone();
      let moderately_loaded_tracker = Arc::new(());
      let _moderate_1 = moderately_loaded_tracker.clone();

      let connection_track = Arc::new(RwLock::new(HashMap::new()));
      {
        let mut connection_track_write = connection_track.write().await;
        connection_track_write.insert(heavily_loaded.clone(), heavily_loaded_tracker);
        connection_track_write.insert(least_loaded.clone(), Arc::new(()));
        connection_track_write.insert(moderately_loaded.clone(), moderately_loaded_tracker);
      }

      let backends = vec![
        (heavily_loaded, None, None),
        (least_loaded, None, None),
        (moderately_loaded, None, None),
      ];
      let algorithm = LoadBalancerAlgorithmInner::LeastConnections(connection_track);

      for _ in 0..32 {
        let selected_index = select_backend_index(&algorithm, &backends).await;
        assert_eq!(selected_index, 1);
      }
    });
  }

  #[test]
  fn determine_proxy_to_skips_unhealthy_backend_when_alternatives_exist() {
    run_async(async {
      let unhealthy = upstream("http://backend-unhealthy");
      let healthy = upstream("http://backend-healthy");
      let mut proxy_to_vector = vec![(unhealthy.clone(), None, None), (healthy.clone(), None, None)];

      let failed_backends = RwLock::new(TtlCache::new(Duration::from_secs(60)));
      {
        let mut failed_backends_write = failed_backends.write().await;
        failed_backends_write.insert(unhealthy, 4);
      }

      let algorithm = LoadBalancerAlgorithmInner::RoundRobin(Arc::new(AtomicUsize::new(0)));
      let selected = determine_proxy_to(&mut proxy_to_vector, &failed_backends, true, 3, &algorithm).await;

      assert!(selected.is_some());
      let (selected_upstream, _, _) = selected.expect("a backend should be selected");
      assert!(selected_upstream == healthy);
    });
  }
}
