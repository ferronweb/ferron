//! DNS client that delegates record changes to an external program.

use std::process::Stdio;

use async_trait::async_trait;
use ferron_dns::{DnsClient, DnsProviderError, DnsRecord, DnsRecordType};

/// DNS client that invokes an external program for each record change.
///
/// The program is run directly (no shell). Record details are passed through
/// environment variables. The program must exit with status `0` to signal
/// success.
pub struct CommandDnsClient {
    program: String,
    min_ttl: u32,
}

impl CommandDnsClient {
    pub fn new(program: String, min_ttl: u32) -> Self {
        Self { program, min_ttl }
    }

    async fn run(
        &self,
        action: &str,
        name: &str,
        record_type: Option<DnsRecordType>,
        value: Option<&str>,
        ttl: Option<u32>,
    ) -> Result<(), DnsProviderError> {
        let mut command = tokio::process::Command::new(&self.program);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        command.env("FERRON_DNS_ACTION", action);
        command.env("FERRON_DNS_DOMAIN", name);

        if let Some(record_type) = record_type {
            command.env("FERRON_DNS_RECORD_TYPE", record_type.to_string());
        }
        if let Some(value) = value {
            command.env("FERRON_DNS_RECORD_VALUE", value);
        }
        if let Some(ttl) = ttl {
            command.env("FERRON_DNS_RECORD_TTL", ttl.to_string());
        }

        let output = command.status().await.map_err(|e| {
            DnsProviderError::new(format!("failed to run DNS command '{}': {e}", self.program))
        })?;

        if output.success() {
            Ok(())
        } else {
            Err(DnsProviderError::new(format!(
                "DNS command '{}' exited with status {} for action '{action}'",
                self.program,
                output
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string())
            )))
        }
    }
}

#[async_trait]
impl DnsClient for CommandDnsClient {
    fn minimum_ttl(&self) -> u32 {
        self.min_ttl
    }

    async fn update_record(&self, record: &DnsRecord) -> Result<(), DnsProviderError> {
        self.run(
            "add",
            &record.name,
            Some(record.record_type),
            Some(record.value.as_str()),
            Some(record.ttl),
        )
        .await
    }

    async fn delete_record(
        &self,
        name: &str,
        record_type: DnsRecordType,
    ) -> Result<(), DnsProviderError> {
        self.run("delete", name, Some(record_type), None, None)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_script(dir: &std::path::Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "#!/bin/sh").unwrap();
        writeln!(file, "{body}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        path.to_string_lossy().to_string()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn update_record_passes_env_and_succeeds() {
        let dir = std::env::temp_dir();
        let log = dir.join("ferron_dns_command_test.log");
        let _ = std::fs::remove_file(&log);
        let script = write_script(
            &dir,
            "ferron_dns_command_ok.sh",
            &format!(
                "echo \"$FERRON_DNS_ACTION|$FERRON_DNS_DOMAIN|$FERRON_DNS_RECORD_TYPE|$FERRON_DNS_RECORD_VALUE|$FERRON_DNS_RECORD_TTL\" >> {}; exit 0",
                log.display()
            ),
        );

        let client = CommandDnsClient::new(script, 60);
        let record = DnsRecord {
            name: "_acme-challenge.example.com".to_string(),
            record_type: DnsRecordType::TXT,
            value: "token-value".to_string(),
            ttl: 120,
        };

        client.update_record(&record).await.unwrap();

        let contents = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            contents.trim(),
            "add|_acme-challenge.example.com|TXT|token-value|120"
        );
        let _ = std::fs::remove_file(&log);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn delete_record_omits_value_and_ttl() {
        let dir = std::env::temp_dir();
        let log = dir.join("ferron_dns_command_del.log");
        let _ = std::fs::remove_file(&log);
        let script = write_script(
            &dir,
            "ferron_dns_command_del.sh",
            &format!(
                "echo \"$FERRON_DNS_ACTION|$FERRON_DNS_RECORD_TYPE|$FERRON_DNS_RECORD_VALUE|$FERRON_DNS_RECORD_TTL\" >> {}; exit 0",
                log.display()
            ),
        );

        let client = CommandDnsClient::new(script, 60);
        client
            .delete_record("_acme-challenge.example.com", DnsRecordType::TXT)
            .await
            .unwrap();

        let contents = std::fs::read_to_string(&log).unwrap();
        assert_eq!(contents.trim(), "delete|TXT||");
        let _ = std::fs::remove_file(&log);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_zero_exit_is_an_error() {
        let dir = std::env::temp_dir();
        let script = write_script(&dir, "ferron_dns_command_fail.sh", "exit 3");
        let client = CommandDnsClient::new(script, 60);
        let record = DnsRecord {
            name: "x.example.com".to_string(),
            record_type: DnsRecordType::TXT,
            value: "v".to_string(),
            ttl: 60,
        };
        let err = client.update_record(&record).await.unwrap_err();
        assert!(err.to_string().contains("exited with status 3"));
    }
}
