pub struct MtlsCertificates(pub Vec<rustls_pki_types::CertificateDer<'static>>);

impl typemap_rev::TypeMapKey for MtlsCertificates {
    type Value = Self;
}
