use rustls_pki_types::{CertificateDer, PrivateKeyDer};

// Load public certificate from file
pub fn load_certs(filename: &str) -> std::io::Result<Vec<CertificateDer<'static>>> {
  let certfile = std::fs::File::open(filename)
    .map_err(|e| std::io::Error::other(format!("failed to open {}: {}", filename, e)))?;
  let mut reader = std::io::BufReader::new(certfile);
  rustls_pemfile::certs(&mut reader).collect()
}

// Load private key from file
pub fn load_private_key(filename: &str) -> std::io::Result<PrivateKeyDer<'static>> {
  let keyfile = std::fs::File::open(filename)
    .map_err(|e| std::io::Error::other(format!("failed to open {}: {}", filename, e)))?;
  let mut reader = std::io::BufReader::new(keyfile);
  match rustls_pemfile::private_key(&mut reader) {
    Ok(Some(private_key)) => Ok(private_key),
    Ok(None) => Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "Invalid private key",
    )),
    Err(err) => Err(err),
  }
}
