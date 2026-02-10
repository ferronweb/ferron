use ferron_common::{get_value, get_values};
use rustls::crypto::aws_lc_rs::cipher_suite::*;
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::crypto::aws_lc_rs::kx_group::*;
use rustls::crypto::CryptoProvider;
use rustls::version::{TLS12, TLS13};
use rustls::{ConfigBuilder, ServerConfig, WantsVerifier, WantsVersions};

/// Initializes the cryptography provider for Rustls.
pub fn init_crypto_provider(
  global_configuration: Option<&ferron_common::config::ServerConfiguration>,
) -> Result<CryptoProvider, anyhow::Error> {
  let mut crypto_provider = default_provider();
  set_cipher_suites(&mut crypto_provider, global_configuration)?;
  set_ecdh_curves(&mut crypto_provider, global_configuration)?;
  Ok(crypto_provider)
}

/// Sets cipher suites based on the global configuration
fn set_cipher_suites(
  crypto_provider: &mut CryptoProvider,
  global_configuration: Option<&ferron_common::config::ServerConfiguration>,
) -> Result<(), anyhow::Error> {
  let cipher_suite: Vec<&ferron_common::config::ServerConfigurationValue> =
    global_configuration.map_or(vec![], |c| get_values!("tls_cipher_suite", c));
  if !cipher_suite.is_empty() {
    let mut cipher_suites = Vec::new();
    let cipher_suite_iter = cipher_suite.iter();
    for cipher_suite_config in cipher_suite_iter {
      if let Some(cipher_suite) = cipher_suite_config.as_str() {
        let cipher_suite_to_add = match cipher_suite {
          "TLS_AES_128_GCM_SHA256" => TLS13_AES_128_GCM_SHA256,
          "TLS_AES_256_GCM_SHA384" => TLS13_AES_256_GCM_SHA384,
          "TLS_CHACHA20_POLY1305_SHA256" => TLS13_CHACHA20_POLY1305_SHA256,
          "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256" => TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
          "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384" => TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
          "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256" => TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
          "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256" => TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
          "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384" => TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
          "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256" => TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
          _ => Err(anyhow::anyhow!(
            "The \"{}\" cipher suite is not supported",
            cipher_suite
          ))?,
        };
        cipher_suites.push(cipher_suite_to_add);
      }
    }
    crypto_provider.cipher_suites = cipher_suites;
  }
  Ok(())
}

/// Sets ECDH curves based on the global configuration.
fn set_ecdh_curves(
  crypto_provider: &mut CryptoProvider,
  global_configuration: Option<&ferron_common::config::ServerConfiguration>,
) -> Result<(), anyhow::Error> {
  let ecdh_curves = global_configuration.map_or(vec![], |c| get_values!("tls_ecdh_curve", c));
  if !ecdh_curves.is_empty() {
    let mut kx_groups = Vec::new();
    let ecdh_curves_iter = ecdh_curves.iter();
    for ecdh_curve_config in ecdh_curves_iter {
      if let Some(ecdh_curve) = ecdh_curve_config.as_str() {
        let kx_group_to_add = match ecdh_curve {
          "secp256r1" => SECP256R1,
          "secp384r1" => SECP384R1,
          "x25519" => X25519,
          "x25519mklem768" => X25519MLKEM768,
          "mklem768" => MLKEM768,
          _ => Err(anyhow::anyhow!("The \"{}\" ECDH curve is not supported", ecdh_curve))?,
        };
        kx_groups.push(kx_group_to_add);
      }
    }
    crypto_provider.kx_groups = kx_groups;
  }
  Ok(())
}

/// Sets the TLS version based on the global configuration.
pub fn set_tls_version(
  tls_config_builder_wants_versions: ConfigBuilder<ServerConfig, WantsVersions>,
  global_configuration: Option<&ferron_common::config::ServerConfiguration>,
) -> Result<ConfigBuilder<ServerConfig, WantsVerifier>, anyhow::Error> {
  let min_tls_version_option = global_configuration
    .and_then(|c| get_value!("tls_min_version", c))
    .and_then(|v| v.as_str());
  let max_tls_version_option = global_configuration
    .and_then(|c| get_value!("tls_max_version", c))
    .and_then(|v| v.as_str());

  let tls_config_builder_wants_verifier = if min_tls_version_option.is_none() && max_tls_version_option.is_none() {
    tls_config_builder_wants_versions.with_safe_default_protocol_versions()?
  } else {
    let tls_versions = [("TLSv1.2", &TLS12), ("TLSv1.3", &TLS13)];
    let min_tls_version_index = min_tls_version_option
      .map_or(Some(0), |v| tls_versions.iter().position(|p| p.0 == v))
      .ok_or(anyhow::anyhow!("Invalid minimum TLS version"))?;
    let max_tls_version_index = max_tls_version_option
      .map_or(Some(tls_versions.len() - 1), |v| {
        tls_versions.iter().position(|p| p.0 == v)
      })
      .ok_or(anyhow::anyhow!("Invalid maximum TLS version"))?;
    if max_tls_version_index < min_tls_version_index {
      Err(anyhow::anyhow!("Maximum TLS version is older than minimum TLS version"))?
    }
    tls_config_builder_wants_versions.with_protocol_versions(
      &tls_versions[min_tls_version_index..=max_tls_version_index]
        .iter()
        .map(|p| p.1)
        .collect::<Vec<_>>(),
    )?
  };

  Ok(tls_config_builder_wants_verifier)
}
