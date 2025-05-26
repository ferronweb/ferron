use std::net::{SocketAddr, TcpStream};

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
  Tcp(TcpStream),

  /// QUIC incoming connection
  Quic(quinn::Incoming),
}
