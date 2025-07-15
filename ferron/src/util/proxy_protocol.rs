// Copyright 2021 Axum Server Contributors
// Portions of this file are derived from `hyper-server` (https://github.com/warlock-labs/postel/tree/6d93b4251766d97120b96ecee6d198b3406da7da).
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.

use std::net::{IpAddr, SocketAddr};

use ppp::HeaderResult;
use tokio::io::{AsyncRead, AsyncReadExt};

/// The length of a v1 header in bytes.
const V1_PREFIX_LEN: usize = 5;
/// The maximum length of a v1 header in bytes.
const V1_MAX_LENGTH: usize = 107;
/// The terminator bytes of a v1 header.
const V1_TERMINATOR: &[u8] = b"\r\n";
/// The prefix length of a v2 header in bytes.
const V2_PREFIX_LEN: usize = 12;
/// The minimum length of a v2 header in bytes.
const V2_MINIMUM_LEN: usize = 16;
/// The index of the start of the big-endian u16 length in the v2 header.
const V2_LENGTH_INDEX: usize = 14;
/// The length of the read buffer used to read the PROXY protocol header.
const READ_BUFFER_LEN: usize = 512;

/// Reads the PROXY protocol header from the given `AsyncRead`.
pub async fn read_proxy_header<I>(mut stream: I) -> Result<(I, Option<SocketAddr>, Option<SocketAddr>), std::io::Error>
where
  I: AsyncRead + Unpin,
{
  // Mutable buffer for storing stream data
  let mut buffer = [0; READ_BUFFER_LEN];
  // Dynamic in case v2 header is too long
  let mut dynamic_buffer = None;

  // Read prefix to check for v1, v2, or kill
  stream.read_exact(&mut buffer[..V1_PREFIX_LEN]).await?;

  if &buffer[..V1_PREFIX_LEN] == ppp::v1::PROTOCOL_PREFIX.as_bytes() {
    read_v1_header(&mut stream, &mut buffer).await?;
  } else {
    stream.read_exact(&mut buffer[V1_PREFIX_LEN..V2_MINIMUM_LEN]).await?;
    if &buffer[..V2_PREFIX_LEN] == ppp::v2::PROTOCOL_PREFIX {
      dynamic_buffer = read_v2_header(&mut stream, &mut buffer).await?;
    } else {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "No valid Proxy Protocol header detected",
      ));
    }
  }

  // Choose which buffer to parse
  let buffer = dynamic_buffer.as_deref().unwrap_or(&buffer[..]);

  // Parse the header
  let header = HeaderResult::parse(buffer);
  match header {
    HeaderResult::V1(Ok(header)) => {
      let (client_address, server_address) = match header.addresses {
        ppp::v1::Addresses::Tcp4(ip) => (
          SocketAddr::new(IpAddr::V4(ip.source_address), ip.source_port),
          SocketAddr::new(IpAddr::V4(ip.destination_address), ip.destination_port),
        ),
        ppp::v1::Addresses::Tcp6(ip) => (
          SocketAddr::new(IpAddr::V6(ip.source_address), ip.source_port),
          SocketAddr::new(IpAddr::V6(ip.destination_address), ip.destination_port),
        ),
        ppp::v1::Addresses::Unknown => {
          // Return client address as `None` so that "unknown" is used in the http header
          return Ok((stream, None, None));
        }
      };

      Ok((stream, Some(client_address), Some(server_address)))
    }
    HeaderResult::V2(Ok(header)) => {
      let (client_address, server_address) = match header.addresses {
        ppp::v2::Addresses::IPv4(ip) => (
          SocketAddr::new(IpAddr::V4(ip.source_address), ip.source_port),
          SocketAddr::new(IpAddr::V4(ip.destination_address), ip.destination_port),
        ),
        ppp::v2::Addresses::IPv6(ip) => (
          SocketAddr::new(IpAddr::V6(ip.source_address), ip.source_port),
          SocketAddr::new(IpAddr::V6(ip.destination_address), ip.destination_port),
        ),
        ppp::v2::Addresses::Unix(unix) => {
          return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Unix socket addresses are not supported. Addresses: {unix:?}"),
          ));
        }
        ppp::v2::Addresses::Unspecified => {
          // Return client address as `None` so that "unknown" is used in the http header
          return Ok((stream, None, None));
        }
      };

      Ok((stream, Some(client_address), Some(server_address)))
    }
    HeaderResult::V1(Err(_error)) => Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "No valid V1 Proxy Protocol header received",
    )),
    HeaderResult::V2(Err(_error)) => Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "No valid V2 Proxy Protocol header received",
    )),
  }
}

async fn read_v2_header<I>(mut stream: I, buffer: &mut [u8; READ_BUFFER_LEN]) -> Result<Option<Vec<u8>>, std::io::Error>
where
  I: AsyncRead + Unpin,
{
  let length = u16::from_be_bytes([buffer[V2_LENGTH_INDEX], buffer[V2_LENGTH_INDEX + 1]]) as usize;
  let full_length = V2_MINIMUM_LEN + length;

  // Switch to dynamic buffer if header is too long; v2 has no maximum length
  if full_length > READ_BUFFER_LEN {
    let mut dynamic_buffer = Vec::with_capacity(full_length);
    dynamic_buffer.extend_from_slice(&buffer[..V2_MINIMUM_LEN]);

    // Read the remaining header length
    stream
      .read_exact(&mut dynamic_buffer[V2_MINIMUM_LEN..full_length])
      .await?;

    Ok(Some(dynamic_buffer))
  } else {
    // Read the remaining header length
    stream.read_exact(&mut buffer[V2_MINIMUM_LEN..full_length]).await?;

    Ok(None)
  }
}

async fn read_v1_header<I>(mut stream: I, buffer: &mut [u8; READ_BUFFER_LEN]) -> Result<(), std::io::Error>
where
  I: AsyncRead + Unpin,
{
  // Read one byte at a time until terminator found
  let mut end_found = false;
  for i in V1_PREFIX_LEN..V1_MAX_LENGTH {
    buffer[i] = stream.read_u8().await?;

    if [buffer[i - 1], buffer[i]] == V1_TERMINATOR {
      end_found = true;
      break;
    }
  }
  if !end_found {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "No valid Proxy Protocol header detected",
    ));
  }

  Ok(())
}
