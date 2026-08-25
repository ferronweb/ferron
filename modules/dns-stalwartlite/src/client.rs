use std::net::AddrParseError;

use ferron_dns::{separate_subdomain_from_domain_name, DnsClient};

pub struct DnsStalwartClient {
    inner: dns_update_lite::DnsUpdater,
    min_ttl: u32,
}

impl DnsStalwartClient {
    pub fn new(inner: dns_update_lite::DnsUpdater, min_ttl: u32) -> Self {
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
            .set_rrset(name, record.as_type(), ttl, vec![record.clone()], &origin)
            .await
            .is_err()
        {
            self.inner
                .add_to_rrset(name, record.as_type(), ttl, vec![record], &origin)
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

        // In `dns_update_lite` 0.5.x, set_rrset with no records deletes the record set.
        self.inner
            .set_rrset(
                name,
                match record_type {
                    ferron_dns::DnsRecordType::A => dns_update_lite::DnsRecordType::A,
                    ferron_dns::DnsRecordType::AAAA => dns_update_lite::DnsRecordType::AAAA,
                    ferron_dns::DnsRecordType::CNAME => dns_update_lite::DnsRecordType::CNAME,
                    ferron_dns::DnsRecordType::TXT => dns_update_lite::DnsRecordType::TXT,
                    ferron_dns::DnsRecordType::MX => dns_update_lite::DnsRecordType::MX,
                    ferron_dns::DnsRecordType::NS => dns_update_lite::DnsRecordType::NS,
                    ferron_dns::DnsRecordType::SRV => dns_update_lite::DnsRecordType::SRV,
                    ferron_dns::DnsRecordType::CAA => dns_update_lite::DnsRecordType::CAA,
                    ferron_dns::DnsRecordType::TLSA => dns_update_lite::DnsRecordType::TLSA,
                    ferron_dns::DnsRecordType::HTTPS => {
                        return Err(ferron_dns::DnsProviderError::new(
                            "HTTPS record type not supported",
                        ))
                    }
                },
                3600,
                vec![],
                origin,
            )
            .await
            .map_err(|e| ferron_dns::DnsProviderError::new(e.to_string()))?;

        Ok(())
    }
}

fn make_dns_record(
    record_type: ferron_dns::DnsRecordType,
    value: String,
) -> Result<dns_update_lite::DnsRecord, ferron_dns::DnsProviderError> {
    Ok(match record_type {
        ferron_dns::DnsRecordType::A => dns_update_lite::DnsRecord::A(
            value
                .parse()
                .map_err(|e: AddrParseError| ferron_dns::DnsProviderError::new(e.to_string()))?,
        ),
        ferron_dns::DnsRecordType::AAAA => dns_update_lite::DnsRecord::AAAA(
            value
                .parse()
                .map_err(|e: AddrParseError| ferron_dns::DnsProviderError::new(e.to_string()))?,
        ),
        ferron_dns::DnsRecordType::CNAME => dns_update_lite::DnsRecord::CNAME(value),
        ferron_dns::DnsRecordType::TXT => dns_update_lite::DnsRecord::TXT(value),
        ferron_dns::DnsRecordType::MX => dns_update_lite::DnsRecord::MX({
            let fields = value
                .split_once(' ')
                .ok_or_else(|| ferron_dns::DnsProviderError::new("invalid MX record"))?;
            let priority = fields
                .0
                .parse()
                .map_err(|_| ferron_dns::DnsProviderError::new("invalid MX priority"))?;
            dns_update_lite::MXRecord {
                exchange: fields.1.to_string(),
                priority,
            }
        }),
        ferron_dns::DnsRecordType::NS => dns_update_lite::DnsRecord::NS(value),
        ferron_dns::DnsRecordType::SRV => dns_update_lite::DnsRecord::SRV({
            let fields = value.split(' ').collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(ferron_dns::DnsProviderError::new("invalid SRV record"));
            }
            let priority = fields[0]
                .parse()
                .map_err(|_| ferron_dns::DnsProviderError::new("invalid SRV record"))?;
            let weight = fields[1]
                .parse()
                .map_err(|_| ferron_dns::DnsProviderError::new("invalid SRV record"))?;
            let port = fields[2]
                .parse()
                .map_err(|_| ferron_dns::DnsProviderError::new("invalid SRV record"))?;
            dns_update_lite::SRVRecord {
                priority,
                weight,
                port,
                target: fields[3].to_string(),
            }
        }),
        ferron_dns::DnsRecordType::CAA => dns_update_lite::DnsRecord::CAA({
            let fields = value
                .split_once(' ')
                .ok_or_else(|| ferron_dns::DnsProviderError::new("invalid CAA record"))?;
            let flags: u8 = fields
                .0
                .parse::<u8>()
                .map_err(|_| ferron_dns::DnsProviderError::new("invalid CAA flags"))?;
            let fields = fields
                .1
                .split_once(' ')
                .ok_or_else(|| ferron_dns::DnsProviderError::new("invalid CAA record"))?;
            match fields.0 {
                "iodef" => dns_update_lite::CAARecord::Iodef {
                    issuer_critical: flags == 128,
                    url: fields.1.to_string(),
                },
                "issue" => dns_update_lite::CAARecord::Issue {
                    issuer_critical: flags == 128,
                    name: Some(fields.1.to_string()),
                    options: vec![],
                },
                "issuewild" => dns_update_lite::CAARecord::IssueWild {
                    issuer_critical: flags == 128,
                    name: Some(fields.1.to_string()),
                    options: vec![],
                },
                _ => return Err(ferron_dns::DnsProviderError::new("invalid CAA record")),
            }
        }),
        ferron_dns::DnsRecordType::TLSA => dns_update_lite::DnsRecord::TLSA({
            let fields = value.split(' ').collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(ferron_dns::DnsProviderError::new("invalid TLSA record"));
            }
            let cert_usage = match fields[0] {
                "0" => dns_update_lite::TlsaCertUsage::PkixTa,
                "1" => dns_update_lite::TlsaCertUsage::PkixEe,
                "2" => dns_update_lite::TlsaCertUsage::DaneTa,
                "3" => dns_update_lite::TlsaCertUsage::DaneEe,
                _ => return Err(ferron_dns::DnsProviderError::new("invalid TLSA record")),
            };
            let selector = match fields[1] {
                "0" => dns_update_lite::TlsaSelector::Full,
                "1" => dns_update_lite::TlsaSelector::Spki,
                _ => return Err(ferron_dns::DnsProviderError::new("invalid TLSA record")),
            };
            let matching = match fields[2] {
                "0" => dns_update_lite::TlsaMatching::Raw,
                "1" => dns_update_lite::TlsaMatching::Sha256,
                "2" => dns_update_lite::TlsaMatching::Sha512,
                _ => return Err(ferron_dns::DnsProviderError::new("invalid TLSA record")),
            };
            // Hex decode the certificate
            let cert_data: Vec<u8> = hex::decode(fields[3])
                .map_err(|_| ferron_dns::DnsProviderError::new("invalid TLSA record"))?;
            dns_update_lite::TLSARecord {
                cert_usage,
                selector,
                matching,
                cert_data,
            }
        }),
        ferron_dns::DnsRecordType::HTTPS => dns_update_lite::DnsRecord::HTTPS({
            let mut fields = value.splitn(3, ' ');
            let svc_priority = fields
                .next()
                .ok_or_else(|| ferron_dns::DnsProviderError::new("invalid HTTPS record"))?
                .parse::<u16>()
                .unwrap_or(0);
            let target_name = fields
                .next()
                .ok_or_else(|| ferron_dns::DnsProviderError::new("invalid HTTPS record"))?
                .to_string();
            let svc_params_orig = fields
                .next()
                .ok_or_else(|| ferron_dns::DnsProviderError::new("invalid HTTPS record"))?
                .to_string();
            let svc_params = svc_params_orig
                .split(" ")
                .map(|s| {
                    let mut parts = s.splitn(2, '=');
                    dns_update_lite::KeyValue {
                        key: parts.next().unwrap_or("").to_string(),
                        value: parts.next().unwrap_or("").trim_matches('"').to_string(),
                    }
                })
                .collect::<Vec<_>>();
            dns_update_lite::HTTPSRecord {
                svc_priority,
                target_name,
                svc_params,
            }
        }),
    })
}
