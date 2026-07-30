use std::sync::Arc;

use crate::config::AcmeConfig;
use crate::emit_log;

/// Cleans up challenge data after certificate issuance.
pub(crate) async fn cleanup_challenge_data(
    config: &AcmeConfig,
    dns_01_domains: &[String],
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
) {
    match config.challenge_type {
        instant_acme::ChallengeType::TlsAlpn01 => {
            *config.tls_alpn_01_data_lock.write().await = None;
        }
        instant_acme::ChallengeType::Http01 => {
            *config.http_01_data_lock.write().await = None;
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
