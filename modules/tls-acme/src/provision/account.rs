use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use hyper::Request;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::rt::TokioExecutor;
use instant_acme::{
    Account, BodyWrapper, BytesResponse, ExternalAccountKey, HttpClient, NewAccount,
};
use rustls::ClientConfig;

use crate::config::AcmeConfig;
use crate::emit_log;

/// Creates a new ACME account and caches it.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_new_account(
    config: &AcmeConfig,
    directory: &str,
    contact: &[String],
    eab_key: Option<&Arc<ExternalAccountKey>>,
    profile: Option<&str>,
    builder: instant_acme::AccountBuilder,
    account_cache_key: &str,
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
) -> Result<Account, Box<dyn std::error::Error + Send + Sync>> {
    let contact_refs: Vec<&str> = contact.iter().map(|s| s.as_str()).collect();
    let (account, credentials) = builder
        .create(
            &NewAccount {
                contact: &contact_refs,
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            directory.to_string(),
            eab_key.map(|e| e.as_ref()),
        )
        .await?;

    if let Err(err) = config
        .account_cache
        .set(account_cache_key, serde_json::to_vec(&credentials)?)
        .await
    {
        emit_log(
            event_sink,
            ferron_observability::LogLevel::Warn,
            "ACME account cache save failed",
            &format!("Failed to save ACME account cache: {}", err),
            "ferron-tls-acme",
            vec![(
                "error.message",
                ferron_observability::LogAttributeValue::String(err.to_string()),
            )],
        );
    }

    let contact = contact
        .first()
        .map(|s| s.as_str())
        .unwrap_or("none")
        .to_string();
    emit_log(
        event_sink,
        ferron_observability::LogLevel::Info,
        "ACME account created",
        &format!(
            "ACME account created for directory {}, contact: {}",
            directory, contact,
        ),
        "ferron-tls-acme",
        vec![
            (
                "ferron.acme.directory",
                ferron_observability::LogAttributeValue::String(directory.to_string()),
            ),
            (
                "ferron.acme.contact",
                ferron_observability::LogAttributeValue::String(contact),
            ),
            (
                "ferron.acme.profile",
                ferron_observability::LogAttributeValue::String(
                    profile.map_or("".to_string(), |p| p.to_string()),
                ),
            ),
        ],
    );

    Ok(account)
}

/// HTTPS client wrapper for instant-acme's HttpClient trait.
pub(crate) struct HttpsClientForAcme(
    HyperClient<hyper_rustls::HttpsConnector<HttpConnector>, BodyWrapper<Bytes>>,
);

impl HttpsClientForAcme {
    pub(crate) fn new(tls_config: ClientConfig) -> Self {
        Self(
            HyperClient::builder(TokioExecutor::new()).build(
                hyper_rustls::HttpsConnectorBuilder::new()
                    .with_tls_config(tls_config)
                    .https_or_http()
                    .enable_http1()
                    .enable_http2()
                    .build(),
            ),
        )
    }
}

impl HttpClient for HttpsClientForAcme {
    fn request(
        &self,
        req: Request<BodyWrapper<Bytes>>,
    ) -> Pin<Box<dyn Future<Output = Result<BytesResponse, instant_acme::Error>> + Send>> {
        HttpClient::request(&self.0, req)
    }
}
