use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};

/// Loads a public certificate from file
pub fn load_certs(filename: &str) -> std::io::Result<Vec<CertificateDer<'static>>> {
  let mut certfile = std::fs::File::open(filename)?;
  CertificateDer::pem_reader_iter(&mut certfile)
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| match e {
      rustls_pki_types::pem::Error::Io(err) => err,
      err => std::io::Error::other(err),
    })
}

/// Loads a private key from file
pub fn load_private_key(filename: &str) -> std::io::Result<PrivateKeyDer<'static>> {
  let mut keyfile = std::fs::File::open(filename)?;
  match PrivateKeyDer::from_pem_reader(&mut keyfile) {
    Ok(private_key) => Ok(private_key),
    Err(rustls_pki_types::pem::Error::Io(err)) => Err(err),
    Err(err) => Err(std::io::Error::other(err)),
  }
}
