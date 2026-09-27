static LISTENER_MUTEX: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(Clone)]
pub struct ListenerMutexGuard {
    _permit: std::sync::Arc<tokio::sync::SemaphorePermit<'static>>,
}

impl ListenerMutexGuard {
    pub async fn acquire() -> Self {
        Self {
            _permit: std::sync::Arc::new(
                LISTENER_MUTEX
                    .acquire()
                    .await
                    .expect("listener mutex acquire failed"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn test_listener_mutex_guard() {
        let guard = ListenerMutexGuard::acquire().await;
        let guard_clone = guard.clone();
        let would_wait = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let ww_clone = would_wait.clone();
        let next_guard_task = tokio::spawn(async move {
            let _ = ListenerMutexGuard::acquire().await;
            if ww_clone.load(std::sync::atomic::Ordering::Relaxed) {
                panic!("should not wait");
            }
        });
        would_wait.store(false, std::sync::atomic::Ordering::Relaxed);
        drop(guard);
        drop(guard_clone);
        next_guard_task.await.unwrap();
    }
}
