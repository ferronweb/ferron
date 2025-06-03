use std::net::SocketAddr;

/// Connection data sent from the listener to the handler
pub struct ConnectionData {
  pub connection: Connection,
  pub client_address: SocketAddr,
  pub server_address: SocketAddr,
}

/// Connection sent from the listener to the handler
#[allow(clippy::large_enum_variant)]
pub enum Connection {
  /// TCP connection
  #[cfg(feature = "runtime-monoio")]
  Tcp(std::net::TcpStream),

  /// TCP connection
  #[cfg(feature = "runtime-tokio")]
  Tcp(tokio::net::TcpStream),

  /// QUIC incoming connection
  Quic(quinn::Incoming),
}
