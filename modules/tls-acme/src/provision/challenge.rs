use std::sync::Arc;

use crate::config::AcmeConfig;
use crate::emit_log;

/// Cleans up challenge data after certificate issuance.
///
/// Clears in-memory locks and removes shared challenge files published for
/// peers (best-effort; expired leftovers are pruned by TTL anyway).
pub(crate) async fn cleanup_challenge_data(
    config: &AcmeConfig,
    dns_01_domains: &[String],
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
) {
    match config.challenge_type {
        instant_acme::ChallengeType::TlsAlpn01 => {
            let identifier = config
                .tls_alpn_01_data_lock
                .write()
                .await
                .take()
                .map(|(_, ident)| ident);
            if let (Some(dir), Some(ident)) =
                (crate::challenge_sync::shared_cache_dir(config), identifier)
            {
                crate::challenge_sync::remove_tlsalpn01_challenge(&dir, &ident).await;
            }
        }
        instant_acme::ChallengeType::Http01 => {
            let token = config
                .http_01_data_lock
                .write()
                .await
                .take()
                .map(|(token, _)| token);
            if let (Some(dir), Some(token)) =
                (crate::challenge_sync::shared_cache_dir(config), token)
            {
                crate::challenge_sync::remove_http01_challenge(&dir, &token).await;
            }
        }
        instant_acme::ChallengeType::Dns01 => {
            if let Some(ref dns_client) = config.dns_client {
                for domain in dns_01_domains {
                    let challenge_domain = format!("_acme-challenge.{domain}");
                    let _ = dns_client
                        .delete_record(&challenge_domain, ferron_dns::DnsRecordType::TXT)
                        .await;
                    emit_log(
                        event_sink,
                        ferron_observability::LogLevel::Debug,
                        "ACME DNS-01 record cleanup",
                        &format!("DNS-01 record cleanup completed for {challenge_domain}"),
                        "ferron-tls-acme",
                        vec![(
                            "ferron.acme.dns_challenge_domain",
                            ferron_observability::LogAttributeValue::String(challenge_domain),
                        )],
                    );
                }
            }
        }
        _ => {}
    }
}
