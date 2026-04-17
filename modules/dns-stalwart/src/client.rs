use std::net::AddrParseError;

use ferron_dns::{separate_subdomain_from_domain_name, DnsClient};

pub struct DnsStalwartClient {
    inner: dns_update::DnsUpdater,
    min_ttl: u32,
}

impl DnsStalwartClient {
    pub fn new(inner: dns_update::DnsUpdater, min_ttl: u32) -> Self {
        Self { inner, min_ttl }
    }
}

#[async_trait::async_trait]
impl DnsClient for DnsStalwartClient {
    fn minimum_ttl(&self) -> u32 {
        self.min_ttl
    }

    async fn update_record(
        &self,
        record: &ferron_dns::DnsRecord,
    ) -> Result<(), ferron_dns::DnsProviderError> {
        let name = &record.name;
        let (_, origin) = separate_subdomain_from_domain_name(name).await;
        let ttl = record.ttl.max(self.min_ttl);
        let record = make_dns_record(record.record_type, record.value.to_string())?;

        if self
            .inner
            .update(name, record.clone(), ttl, &origin)
            .await
            .is_err()
        {
            self.inner
                .create(name, record, ttl, &origin)
                .await
                .map_err(|e| ferron_dns::DnsProviderError::new(e.to_string()))?;
        }

        Ok(())
    }

    async fn delete_record(
        &self,
        name: &str,
        record_type: ferron_dns::DnsRecordType,
    ) -> Result<(), ferron_dns::DnsProviderError> {
        let (_, origin) = separate_subdomain_from_domain_name(name).await;

        self.inner
            .delete(
                name,
                origin,
                match record_type {
                    ferron_dns::DnsRecordType::A => dns_update::DnsRecordType::A,
                    ferron_dns::DnsRecordType::AAAA => dns_update::DnsRecordType::AAAA,
                    ferron_dns::DnsRecordType::CNAME => dns_update::DnsRecordType::CNAME,
                    ferron_dns::DnsRecordType::TXT => dns_update::DnsRecordType::TXT,
                    ferron_dns::DnsRecordType::MX => dns_update::DnsRecordType::MX,
                    ferron_dns::DnsRecordType::NS => dns_update::DnsRecordType::NS,
                    ferron_dns::DnsRecordType::SRV => dns_update::DnsRecordType::SRV,
                    ferron_dns::DnsRecordType::CAA => dns_update::DnsRecordType::CAA,
                    ferron_dns::DnsRecordType::TLSA => dns_update::DnsRecordType::TLSA,
                    ferron_dns::DnsRecordType::HTTPS => {
                        return Err(ferron_dns::DnsProviderError::new(
                            "HTTPS record type not supported",
                        ))
                    }
                },
            )
            .await
            .map_err(|e| ferron_dns::DnsProviderError::new(e.to_string()))?;

        Ok(())
    }
}

fn make_dns_record(
    record_type: ferron_dns::DnsRecordType,
    value: String,
) -> Result<dns_update::DnsRecord, ferron_dns::DnsProviderError> {
    Ok(match record_type {
        ferron_dns::DnsRecordType::A => dns_update::DnsRecord::A(
            value
                .parse()
                .map_err(|e: AddrParseError| ferron_dns::DnsProviderError::new(e.to_string()))?,
        ),
        ferron_dns::DnsRecordType::AAAA => dns_update::DnsRecord::AAAA(
            value
                .parse()
                .map_err(|e: AddrParseError| ferron_dns::DnsProviderError::new(e.to_string()))?,
        ),
        ferron_dns::DnsRecordType::CNAME => dns_update::DnsRecord::CNAME(value),
        ferron_dns::DnsRecordType::TXT => dns_update::DnsRecord::TXT(value),
        ferron_dns::DnsRecordType::MX => dns_update::DnsRecord::MX({
            let fields = value
                .split_once(' ')
                .ok_or_else(|| ferron_dns::DnsProviderError::new("invalid MX record"))?;
            dns_update::MXRecord {
                exchange: fields.1.to_string(),
                priority: fields.0.parse().unwrap_or(0),
            }
        }),
        ferron_dns::DnsRecordType::NS => dns_update::DnsRecord::NS(value),
        ferron_dns::DnsRecordType::SRV => dns_update::DnsRecord::SRV({
            let fields = value.split(' ').collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(ferron_dns::DnsProviderError::new("invalid SRV record"));
            }
            dns_update::SRVRecord {
                priority: fields[0].parse().unwrap_or(0),
                weight: fields[1].parse().unwrap_or(0),
                port: fields[2].parse().unwrap_or(0),
                target: fields[3].to_string(),
            }
        }),
        ferron_dns::DnsRecordType::CAA => dns_update::DnsRecord::CAA({
            let fields = value
                .split_once(' ')
                .ok_or_else(|| ferron_dns::DnsProviderError::new("invalid CAA record"))?;
            let flags = fields.0.parse().unwrap_or(0);
            let fields = fields
                .1
                .split_once(' ')
                .ok_or_else(|| ferron_dns::DnsProviderError::new("invalid CAA record"))?;
            match fields.0 {
                "iodef" => dns_update::CAARecord::Iodef {
                    issuer_critical: flags == 128,
                    url: fields.1.to_string(),
                },
                "issue" => dns_update::CAARecord::Issue {
                    issuer_critical: flags == 128,
                    name: Some(fields.1.to_string()),
                    options: vec![],
                },
                "issuewild" => dns_update::CAARecord::IssueWild {
                    issuer_critical: flags == 128,
                    name: Some(fields.1.to_string()),
                    options: vec![],
                },
                _ => return Err(ferron_dns::DnsProviderError::new("invalid CAA record")),
            }
        }),
        ferron_dns::DnsRecordType::TLSA => dns_update::DnsRecord::TLSA({
            let fields = value.split(' ').collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(ferron_dns::DnsProviderError::new("invalid TLSA record"));
            }
            let cert_usage = match fields[0] {
                "0" => dns_update::TlsaCertUsage::PkixTa,
                "1" => dns_update::TlsaCertUsage::PkixEe,
                "2" => dns_update::TlsaCertUsage::DaneTa,
                "3" => dns_update::TlsaCertUsage::DaneEe,
                _ => return Err(ferron_dns::DnsProviderError::new("invalid TLSA record")),
            };
            let selector = match fields[1] {
                "0" => dns_update::TlsaSelector::Full,
                "1" => dns_update::TlsaSelector::Spki,
                _ => return Err(ferron_dns::DnsProviderError::new("invalid TLSA record")),
            };
            let matching = match fields[2] {
                "0" => dns_update::TlsaMatching::Raw,
                "1" => dns_update::TlsaMatching::Sha256,
                "2" => dns_update::TlsaMatching::Sha512,
                _ => return Err(ferron_dns::DnsProviderError::new("invalid TLSA record")),
            };
            // Hex decode the certificate
            let cert_data: Vec<u8> = hex::decode(fields[3])
                .map_err(|_| ferron_dns::DnsProviderError::new("invalid TLSA record"))?;
            dns_update::TLSARecord {
                cert_usage,
                selector,
                matching,
                cert_data,
            }
        }),
        _ => {
            return Err(ferron_dns::DnsProviderError::new(format!(
                "Unsupported DNS record type: {record_type}"
            )))
        }
    })
}
