use std::sync::Arc;

use rustls::sign::CertifiedKey;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::AsyncWriteExt;

use crate::cache::CertificateCacheData;
use crate::config::AcmeConfig;
use crate::emit_log;

/// Installs a certified key into the config and optionally saves to disk.
pub(crate) async fn install_certified_key(
    config: &AcmeConfig,
    certs: Vec<CertificateDer<'static>>,
    private_key: PrivateKeyDer<'static>,
    cache_data: &CertificateCacheData,
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let domains = config.domains.join(", ");
    let chain_len = certs.len();

    let signing_key = rustls::crypto::aws_lc_rs::default_provider()
        .key_provider
        .load_private_key(private_key)?;

    *config.certified_key_lock.write().await =
        Some(Arc::new(CertifiedKey::new(certs, signing_key)));

    // Emit the unified `ferron.tls.certificate_not_after` gauge for the leaf
    // of the just-mounted chain.
    if let Some(leaf) = config
        .certified_key_lock
        .read()
        .await
        .as_deref()
        .and_then(|ck| ck.cert.first())
    {
        ferron_tls::observability::emit_certificate_not_after(
            event_sink,
            "acme",
            config.domains.first().map(String::as_str).unwrap_or(""),
            leaf,
        );
    }

    emit_log(
        event_sink,
        ferron_observability::LogLevel::Debug,
        "ACME certificate installed",
        &format!("Certificate installed for {domains}, chain length: {chain_len}"),
        "ferron-tls-acme",
        vec![
            (
                "ferron.acme.domains",
                ferron_observability::LogAttributeValue::String(domains.clone()),
            ),
            (
                "ferron.acme.chain_length",
                ferron_observability::LogAttributeValue::I64(chain_len as i64),
            ),
        ],
    );

    // Save to files if configured
    if let Some((cert_path, key_path)) = &config.save_paths {
        tokio::fs::write(cert_path, &cache_data.certificate_chain_pem).await?;

        let mut open_options = tokio::fs::OpenOptions::new();
        open_options.write(true).create(true).truncate(true);

        #[cfg(unix)]
        open_options.mode(0o600);

        let mut file = open_options.open(key_path).await?;
        file.write_all(cache_data.private_key_pem.as_bytes())
            .await?;
        file.flush().await.unwrap_or_default();

        if let Some(command) = &config.post_obtain_command {
            emit_log(
                event_sink,
                ferron_observability::LogLevel::Info,
                "ACME post-obtain command started",
                &format!("Post-obtain command started for {domains}"),
                "ferron-tls-acme",
                vec![(
                    "ferron.acme.domains",
                    ferron_observability::LogAttributeValue::String(domains.clone()),
                )],
            );

            let Some(parts) = shlex::split(command) else {
                emit_log(
                    event_sink,
                    ferron_observability::LogLevel::Warn,
                    "ACME post-obtain command malformed",
                    &format!("Post-obtain command has malformed quoting for {domains}"),
                    "ferron-tls-acme",
                    vec![(
                        "ferron.acme.domains",
                        ferron_observability::LogAttributeValue::String(domains.clone()),
                    )],
                );
                return Ok(());
            };

            if let Some((program, args)) = parts.split_first() {
                let mut cmd = tokio::process::Command::new(program);
                for arg in args {
                    cmd.arg(arg);
                }
                cmd.env("FERRON_ACME_DOMAIN", config.domains.join(","))
                    .env("FERRON_ACME_CERT_PATH", cert_path)
                    .env("FERRON_ACME_KEY_PATH", key_path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());

                match cmd.spawn() {
                    Ok(mut child) => {
                        let _ = child.wait().await;
                    }
                    Err(e) => {
                        emit_log(
                            event_sink,
                            ferron_observability::LogLevel::Warn,
                            "ACME post-obtain command failed",
                            &format!("Post-obtain command failed for {domains}: {e}"),
                            "ferron-tls-acme",
                            vec![
                                (
                                    "ferron.acme.domains",
                                    ferron_observability::LogAttributeValue::String(
                                        domains.clone(),
                                    ),
                                ),
                                (
                                    "error.message",
                                    ferron_observability::LogAttributeValue::String(e.to_string()),
                                ),
                            ],
                        );
                    }
                }
            } else {
                emit_log(
                    event_sink,
                    ferron_observability::LogLevel::Warn,
                    "ACME post-obtain command empty",
                    &format!("Post-obtain command is empty for {domains}"),
                    "ferron-tls-acme",
                    vec![(
                        "ferron.acme.domains",
                        ferron_observability::LogAttributeValue::String(domains.clone()),
                    )],
                );
            }
        }
    }

    Ok(())
}
