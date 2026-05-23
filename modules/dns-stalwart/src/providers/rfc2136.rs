use std::net::ToSocketAddrs;
use std::sync::Arc;

use base64::Engine;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct Rfc2136DnsProvider;

impl Provider<DnsContext<'static>> for Rfc2136DnsProvider {
    fn name(&self) -> &'static str {
        "rfc2136"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let addr_str = required_string(ctx, "server", "rfc2136", "server address")?;
        let url: hyper::Uri = addr_str
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid RFC 2136 server address: {e}"))?;

        let resolve = |scheme: &str| -> Result<_, anyhow::Error> {
            let host = url
                .authority()
                .map(|a| a.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing RFC 2136 server address hostname"))?;
            let addr = host
                .to_socket_addrs()
                .map_err(|e| anyhow::anyhow!("Failed to resolve RFC 2136 server address: {e}"))?
                .next()
                .ok_or_else(|| anyhow::anyhow!("No RFC 2136 server addresses found"))?;
            match scheme {
                "tcp" => Ok(dns_update::providers::rfc2136::DnsAddress::Tcp(addr)),
                "udp" => Ok(dns_update::providers::rfc2136::DnsAddress::Udp(addr)),
                _ => Err(anyhow::anyhow!("Invalid RFC 2136 server address scheme")),
            }
        };

        let addr = match url.scheme().map(|s| s.as_str()) {
            Some(s @ ("tcp" | "udp")) => resolve(s)?,
            _ => Err(anyhow::anyhow!("Invalid RFC 2136 server address scheme"))?,
        };

        let key_name = required_string(ctx, "key_name", "rfc2136", "key name")?;
        let key = base64::engine::general_purpose::STANDARD
            .decode(
                required_string(ctx, "key_secret", "rfc2136", "key secret")?
                    .as_bytes(),
            )
            .map_err(|e| anyhow::anyhow!("Failed to decode RFC 2136 key: {e}"))?;

        let tsig_algorithm = match required_string(ctx, "key_algorithm", "rfc2136", "TSIG algorithm")?
            .to_uppercase()
            .as_str()
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
