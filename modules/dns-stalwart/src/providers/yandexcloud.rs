use std::collections::HashMap;
use std::sync::Arc;

use dns_update::providers::yandexcloud::YandexCloudConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct YandexCloudDnsProvider;

impl Provider<DnsContext<'static>> for YandexCloudDnsProvider {
    fn name(&self) -> &'static str {
        "yandexcloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let iam_token_b64 = ctx
            .config
            .get_value("iam_token_b64")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid iam_token_b64 for 'yandexcloud' DNS provider"
            ))?;

        let folder_id = ctx
            .config
            .get_value("folder_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid folder_id for 'yandexcloud' DNS provider"
            ))?;

        let config = YandexCloudConfig {
            iam_token_b64,
            folder_id,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_yandexcloud(config)?,
            0,
        )));
        Ok(())
    }
}
