use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::DigitallySignedStruct;
use rustls::SignatureScheme::{self, *};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};

#[derive(Debug)]
pub struct NoServerVerifier;

impl NoServerVerifier {
  pub fn new() -> Self {
    NoServerVerifier
  }
}

impl ServerCertVerifier for NoServerVerifier {
  fn verify_server_cert(
    &self,
    _end_entity: &CertificateDer<'_>,
    _intermediates: &[CertificateDer<'_>],
    _server_name: &ServerName<'_>,
    _ocsp_response: &[u8],
    _now: UnixTime,
  ) -> Result<ServerCertVerified, rustls::Error> {
    Ok(ServerCertVerified::assertion())
  }

  fn verify_tls12_signature(
    &self,
    _message: &[u8],
    _cert: &CertificateDer<'_>,
    _dss: &DigitallySignedStruct,
  ) -> Result<HandshakeSignatureValid, rustls::Error> {
    Ok(HandshakeSignatureValid::assertion())
  }

  fn verify_tls13_signature(
    &self,
    _message: &[u8],
    _cert: &CertificateDer<'_>,
    _dss: &DigitallySignedStruct,
  ) -> Result<HandshakeSignatureValid, rustls::Error> {
    Ok(HandshakeSignatureValid::assertion())
  }

  fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
    // Extend the list when necessary
    vec![
      ECDSA_NISTP384_SHA384,
      ECDSA_NISTP256_SHA256,
      ED25519,
      RSA_PSS_SHA512,
      RSA_PSS_SHA384,
      RSA_PSS_SHA256,
      RSA_PKCS1_SHA512,
      RSA_PKCS1_SHA384,
      RSA_PKCS1_SHA256,
    ]
  }
}
