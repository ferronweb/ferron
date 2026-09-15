static LISTENER_MUTEX: std::sync::Mutex<Option<tokio_util::sync::CancellationToken>> =
    std::sync::Mutex::new(None);

#[derive(Clone)]
pub struct ListenerMutexGuard {
    rc: std::sync::Arc<()>,
}

impl ListenerMutexGuard {
    pub async fn acquire() -> Self {
        let token = match &mut *LISTENER_MUTEX.lock().expect("listener mutex lock failed") {
            Some(token) => Some(token.clone()),
            token @ None => {
                *token = Some(tokio_util::sync::CancellationToken::new());
                None
            }
        };
        if let Some(token) = token {
            token.cancelled().await;
        }
        Self {
            rc: std::sync::Arc::new(()),
        }
    }
}

impl Drop for ListenerMutexGuard {
    fn drop(&mut self) {
        if std::sync::Arc::get_mut(&mut self.rc).is_some() {
            let opt = &mut *LISTENER_MUTEX.lock().expect("listener mutex lock failed");
            if let Some(token) = opt {
                token.cancel();
            }
            *opt = None;
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
