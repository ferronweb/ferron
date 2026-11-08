use std::sync::Arc;
use std::time::Duration;

use crate::config::AcmeConfig;
use crate::emit_log;

/// Longest wait for a DNS-01 record to propagate (the former fixed delay).
const DNS_PROPAGATION_TIMEOUT: Duration = Duration::from_secs(60);
/// How often the authoritative nameservers are queried while waiting.
const DNS_PROPAGATION_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Waits until the zone's authoritative nameservers serve a DNS-01 record.
///
/// Gives up after [`DNS_PROPAGATION_TIMEOUT`] and lets the CA validate anyway.
/// When the nameservers can't be resolved (for example for a private zone),
/// it waits the full timeout instead.
pub(crate) async fn wait_for_dns_propagation(
    challenge_domain: &str,
    value: &str,
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
) {
    let deadline = tokio::time::Instant::now() + DNS_PROPAGATION_TIMEOUT;
    let attributes = || {
        vec![(
            "ferron.acme.dns_challenge_domain",
            ferron_observability::LogAttributeValue::String(challenge_domain.to_string()),
        )]
    };

    let check = tokio::time::timeout_at(
        deadline,
        ferron_dns::AuthoritativeTxtCheck::new(challenge_domain),
    )
    .await;
    let Ok(Some(check)) = check else {
        emit_log(
            event_sink,
            ferron_observability::LogLevel::Debug,
            "ACME DNS-01 propagation unchecked",
            &format!(
                "Couldn't resolve the authoritative nameservers for {challenge_domain}, \
                 waiting out the {}-second DNS propagation delay",
                DNS_PROPAGATION_TIMEOUT.as_secs()
            ),
            "ferron-tls-acme",
            attributes(),
        );
        tokio::time::sleep_until(deadline).await;
        return;
    };

    let served = tokio::time::timeout_at(deadline, async {
        while !check.is_served(value).await {
            tokio::time::sleep(DNS_PROPAGATION_POLL_INTERVAL).await;
        }
    })
    .await
    .is_ok();
    if served {
        emit_log(
            event_sink,
            ferron_observability::LogLevel::Debug,
            "ACME DNS-01 record propagated",
            &format!(
                "DNS-01 record for {challenge_domain} is served by all authoritative nameservers"
            ),
            "ferron-tls-acme",
            attributes(),
        );
    } else {
        emit_log(
            event_sink,
            ferron_observability::LogLevel::Warn,
            "ACME DNS-01 record not propagated",
            &format!(
                "DNS-01 record for {challenge_domain} isn't served by all authoritative \
                 nameservers after {} seconds, requesting validation anyway",
                DNS_PROPAGATION_TIMEOUT.as_secs()
            ),
            "ferron-tls-acme",
            attributes(),
        );
    }
}

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
