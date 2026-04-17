use std::net::ToSocketAddrs;
use std::{collections::HashMap, sync::Arc};

use base64::Engine;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct Rfc2136DnsProvider;

impl Provider<DnsContext<'static>> for Rfc2136DnsProvider {
    fn name(&self) -> &'static str {
        "rfc2136"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let addr_str = ctx
            .config
            .get_value("server")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid server address for 'rfc2136' DNS provider"
            ))?;

        let url: hyper::Uri = addr_str
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid RFC 2136 server address: {}", e))?;

        let addr = match url.scheme().map(|s| s.as_str()) {
            Some("tcp") => dns_update::providers::rfc2136::DnsAddress::Tcp(
                url.host()
                    .ok_or_else(|| anyhow::anyhow!("Missing RFC 2136 server address hostname"))?
                    .to_socket_addrs()
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to resolve RFC 2136 server address: {}", e)
                    })?
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("No RFC 2136 server addresses found"))?,
            ),
            Some("udp") => dns_update::providers::rfc2136::DnsAddress::Udp(
                url.host()
                    .ok_or_else(|| anyhow::anyhow!("Missing RFC 2136 server address hostname"))?
                    .to_socket_addrs()
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to resolve RFC 2136 server address: {}", e)
                    })?
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("No RFC 2136 server addresses found"))?,
            ),
            _ => Err(anyhow::anyhow!("Invalid RFC 2136 server address scheme"))?,
        };

        let key_name = ctx
            .config
            .get_value("key_name")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid key name for 'rfc2136' DNS provider"
            ))?;

        let key = base64::engine::general_purpose::STANDARD
            .decode(
                ctx.config
                    .get_value("key_secret")
                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                    .ok_or(anyhow::anyhow!(
                        "Missing or invalid key secret for 'rfc2136' DNS provider"
                    ))?,
            )
            .map_err(|e| anyhow::anyhow!("Failed to decode RFC 2136 key: {}", e))?;

        let tsig_algorithm = match &ctx
            .config
            .get_value("key_algorithm")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid TSIG algorithm for 'rfc2136' DNS provider"
            ))?
            .to_uppercase() as &str
        {
            "HMAC-MD5" => dns_update::TsigAlgorithm::HmacMd5,
            "GSS" => dns_update::TsigAlgorithm::Gss,
            "HMAC-SHA1" => dns_update::TsigAlgorithm::HmacSha1,
            "HMAC-SHA224" => dns_update::TsigAlgorithm::HmacSha224,
            "HMAC-SHA256" => dns_update::TsigAlgorithm::HmacSha256,
            "HMAC-SHA256-128" => dns_update::TsigAlgorithm::HmacSha256_128,
            "HMAC-SHA384" => dns_update::TsigAlgorithm::HmacSha384,
            "HMAC-SHA384-192" => dns_update::TsigAlgorithm::HmacSha384_192,
            "HMAC-SHA512" => dns_update::TsigAlgorithm::HmacSha512,
            "HMAC-SHA512-256" => dns_update::TsigAlgorithm::HmacSha512_256,
            _ => Err(anyhow::anyhow!("Unsupported RFC 2136 TSIG algorithm"))?,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_rfc2136_tsig(addr, &key_name, key, tsig_algorithm)?,
            1,
        )));
        Ok(())
    }
}
