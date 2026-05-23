use std::sync::Arc;

use dns_update::providers::yandexcloud::YandexCloudConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct YandexCloudDnsProvider;

impl Provider<DnsContext<'static>> for YandexCloudDnsProvider {
    fn name(&self) -> &'static str {
        "yandexcloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let config = YandexCloudConfig {
            iam_token_b64: required_string(ctx, "iam_token_b64", "yandexcloud", "IAM token (base64)")?,
            folder_id: required_string(ctx, "folder_id", "yandexcloud", "folder ID")?,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_yandexcloud(config)?,
            0,
        )));
        Ok(())
    }
}
