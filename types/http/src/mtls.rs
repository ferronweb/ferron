//! Mutual TLS (mTLS) client certificate storage.
//!
//! When mTLS is enabled and the client presents a certificate, the
//! certificate chain is stored in [`HttpContext::extensions`](crate::HttpContext::extensions)
//! as [`MtlsCertificates`]. Modules can retrieve it to access client
//! certificate details (e.g. Common Name for authentication).

/// The client certificate chain from a mutual TLS handshake.
///
/// Stored in [`HttpContext::extensions`](crate::HttpContext::extensions) when
/// mTLS is enabled and the client presents a certificate.
pub struct MtlsCertificates(pub Vec<rustls_pki_types::CertificateDer<'static>>);

impl typemap_rev::TypeMapKey for MtlsCertificates {
    /// The stored value is the certificate chain itself.
    type Value = Self;
}
