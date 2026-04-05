//! HTTP-01 ACME challenge stage.
//!
//! Intercepts requests to `/.well-known/acme-challenge/<token>` and serves
//! the corresponding key authorization from the shared ACME challenge locks.

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::{HttpContext, HttpResponse};
use http::Response;
use http_body_util::{BodyExt, Full};

use crate::challenge::http01::try_handle_challenge;
use crate::get_or_init_task_state;

/// Stage that handles HTTP-01 ACME challenge requests.
pub struct AcmeHttp01ChallengeStage;

impl Default for AcmeHttp01ChallengeStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for AcmeHttp01ChallengeStage {
    #[inline]
    fn name(&self) -> &str {
        "acme_http01"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        // Run very early in the pipeline, before other request handlers
        vec![StageConstraint::Before("hello".to_string())]
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let Some(req) = ctx.req.as_ref() else {
            return Ok(true);
        };

        let path = req.uri().path();

        // Only handle ACME challenge paths
        if !path.starts_with("/.well-known/acme-challenge/") {
            return Ok(true);
        }

        // Look up the key authorization from the shared ACME state
        let task_state = get_or_init_task_state();
        let resolvers = task_state.http_01_resolvers.blocking_read();

        if let Some(key_authorization) = try_handle_challenge(path, &resolvers) {
            ctx.res = Some(HttpResponse::Custom(
                Response::builder()
                    .status(200)
                    .header("Content-Type", "text/plain")
                    .header("Content-Length", key_authorization.len())
                    .body(
                        Full::new(Bytes::from(key_authorization))
                            .map_err(|e| match e {})
                            .boxed_unsync(),
                    )
                    .expect("Failed to build ACME challenge response"),
            ));
            return Ok(false);
        }

        // No matching token found — let the pipeline continue (will eventually 404)
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::challenge::Http01DataLock;
    use crate::AcmeTaskState;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[test]
    fn test_challenge_path_detection() {
        assert!("/.well-known/acme-challenge/token123".starts_with("/.well-known/acme-challenge/"));
        assert!(!"/other/path".starts_with("/.well-known/acme-challenge/"));
    }

    #[test]
    fn test_try_handle_challenge_matches_token() {
        let lock: Http01DataLock = Arc::new(RwLock::new(Some((
            "mytoken".to_string(),
            "mykeyauth".to_string(),
        ))));
        let resolvers = vec![lock];
        let result = try_handle_challenge("/.well-known/acme-challenge/mytoken", &resolvers);
        assert_eq!(result, Some("mykeyauth".to_string()));
    }

    #[test]
    fn test_try_handle_challenge_wrong_token() {
        let lock: Http01DataLock = Arc::new(RwLock::new(Some((
            "mytoken".to_string(),
            "mykeyauth".to_string(),
        ))));
        let resolvers = vec![lock];
        let result = try_handle_challenge("/.well-known/acme-challenge/othertoken", &resolvers);
        assert!(result.is_none());
    }

    #[test]
    fn test_task_state_has_http01_resolvers() {
        let state = AcmeTaskState::new();
        assert!(state.http_01_resolvers.blocking_read().is_empty());
    }
}
