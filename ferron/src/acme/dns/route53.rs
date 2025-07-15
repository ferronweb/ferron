use std::{error::Error, sync::Arc};

use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_route53::{
  types::{Change, ChangeAction, ChangeBatch, ResourceRecord, ResourceRecordSet, RrType},
  Client,
};
use tokio::sync::Mutex;

use crate::acme::dns::{separate_subdomain_from_domain_name, DnsProvider};

/// Amazon Route 53 DNS provider
pub struct Route53DnsProvider {
  region: Option<String>,
  profile_name: Option<String>,
  credentials: Option<Credentials>,
  hosted_zone_id: Option<String>,
  client: Mutex<Option<Arc<Client>>>,
}

impl Route53DnsProvider {
  /// Create a new Route53 DNS provider
  pub fn new(
    region: Option<&str>,
    profile_name: Option<&str>,
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
    hosted_zone_id: Option<&str>,
  ) -> Result<Self, anyhow::Error> {
    if access_key_id.is_some() && secret_access_key.is_none() {
      return Err(anyhow::anyhow!(
        "secret_access_key is required when access_key_id is provided"
      ));
    } else if access_key_id.is_none() && secret_access_key.is_some() {
      return Err(anyhow::anyhow!(
        "access_key_id is required when secret_access_key is provided"
      ));
    }
    let mut credentials = None;
    if let Some(access_key_id) = access_key_id {
      if let Some(secret_access_key) = secret_access_key {
        credentials = Some(Credentials::from_keys(access_key_id, secret_access_key, None))
      }
    }
    Ok(Self {
      region: region.map(|r| r.to_string()),
      profile_name: profile_name.map(|p| p.to_string()),
      credentials,
      hosted_zone_id: hosted_zone_id.map(|h| h.to_string()),
      client: Mutex::new(None),
    })
  }
}

#[async_trait]
impl DnsProvider for Route53DnsProvider {
  async fn set_acme_txt_record(
    &self,
    acme_challenge_identifier: &str,
    dns_value: &str,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    let client_option = &mut *self.client.lock().await;
    let client = if let Some(client) = client_option {
      client.clone()
    } else {
      let mut config_loader = aws_config::defaults(BehaviorVersion::latest());
      if let Some(region) = &self.region {
        config_loader = config_loader.region(Region::new(region.to_string()));
      }
      if let Some(profile_name) = &self.profile_name {
        config_loader = config_loader.profile_name(profile_name.to_string());
      }
      if let Some(credentials) = &self.credentials {
        config_loader = config_loader.credentials_provider(credentials.to_owned());
      }
      let config = config_loader.load().await;
      let client = Arc::new(Client::new(&config));
      client_option.replace(client.clone());
      client
    };
    let (subdomain, domain_name) = separate_subdomain_from_domain_name(acme_challenge_identifier).await;
    let subdomain = if subdomain.is_empty() {
      "_acme-challenge".to_string()
    } else {
      format!("_acme-challenge.{subdomain}")
    };
    let hosted_zone_id = if let Some(hosted_zone_id) = &self.hosted_zone_id {
      hosted_zone_id.to_string()
    } else {
      let hosted_zones = client.list_hosted_zones_by_name().dns_name(&domain_name).send().await?;
      hosted_zones
        .hosted_zone_id()
        .ok_or_else(|| anyhow::anyhow!("Route 53 hosted zone not found"))?
        .to_string()
    };
    client
      .change_resource_record_sets()
      .hosted_zone_id(hosted_zone_id)
      .change_batch(
        ChangeBatch::builder()
          .changes(
            Change::builder()
              .action(ChangeAction::Create)
              .resource_record_set(
                ResourceRecordSet::builder()
                  .name(format!("{subdomain}.{domain_name}."))
                  .r#type(RrType::Txt)
                  .ttl(300)
                  .resource_records(ResourceRecord::builder().value(format!("\"{dns_value}\"")).build()?)
                  .build()?,
              )
              .build()?,
          )
          .build()?,
      )
      .send()
      .await?;
    Ok(())
  }

  async fn remove_acme_txt_record(&self, acme_challenge_identifier: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let client_option = &mut *self.client.lock().await;
    let client = if let Some(client) = client_option {
      client.clone()
    } else {
      let mut config_loader = aws_config::defaults(BehaviorVersion::latest());
      if let Some(region) = &self.region {
        config_loader = config_loader.region(Region::new(region.to_string()));
      }
      if let Some(profile_name) = &self.profile_name {
        config_loader = config_loader.profile_name(profile_name.to_string());
      }
      if let Some(credentials) = &self.credentials {
        config_loader = config_loader.credentials_provider(credentials.to_owned());
      }
      let config = config_loader.load().await;
      let client = Arc::new(Client::new(&config));
      client_option.replace(client.clone());
      client
    };
    let (subdomain, domain_name) = separate_subdomain_from_domain_name(acme_challenge_identifier).await;
    let subdomain = if subdomain.is_empty() {
      "_acme-challenge".to_string()
    } else {
      format!("_acme-challenge.{subdomain}")
    };
    let hosted_zone_id = if let Some(hosted_zone_id) = &self.hosted_zone_id {
      hosted_zone_id.to_string()
    } else {
      let hosted_zones = client.list_hosted_zones_by_name().dns_name(&domain_name).send().await?;
      hosted_zones
        .hosted_zone_id()
        .ok_or_else(|| anyhow::anyhow!("Route 53 hosted zone not found"))?
        .to_string()
    };
    client
      .change_resource_record_sets()
      .hosted_zone_id(hosted_zone_id)
      .change_batch(
        ChangeBatch::builder()
          .changes(
            Change::builder()
              .action(ChangeAction::Delete)
              .resource_record_set(
                ResourceRecordSet::builder()
                  .name(format!("{subdomain}.{domain_name}."))
                  .r#type(RrType::Txt)
                  .build()?,
              )
              .build()?,
          )
          .build()?,
      )
      .send()
      .await?;
    Ok(())
  }
}
