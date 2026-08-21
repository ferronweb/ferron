use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// Oracle Cloud DNS provider
  OracleCloudDnsProvider,
  "Oracle Cloud",
  |challenge_params| dns_update::DnsUpdater::new_oraclecloud(dns_update::providers::oraclecloud::OracleCloudConfig {
    tenancy_ocid: require_param(challenge_params, "tenancy_ocid", "Oracle Cloud tenancy OCID")?.to_string(),
    user_ocid: require_param(challenge_params, "user_ocid", "Oracle Cloud user OCID")?.to_string(),
    fingerprint: require_param(challenge_params, "fingerprint", "Oracle Cloud key fingerprint")?.to_string(),
    private_key_pem: require_param(challenge_params, "private_key_pem", "Oracle Cloud private key PEM")?.to_string(),
    private_key_password: optional_param(challenge_params, "private_key_password").map(String::from),
    region: require_param(challenge_params, "region", "Oracle Cloud region")?.to_string(),
    compartment_ocid: require_param(challenge_params, "compartment_ocid", "Oracle Cloud compartment OCID")?.to_string(),
    request_timeout: None,
  })
);
