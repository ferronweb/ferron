//! Connection establishment logic for the proxy.

use std::net::IpAddr;

use crate::types::upstream::ProxyHeader;

/// Build a PROXY protocol header for the given version and connection details.
pub fn build_proxy_protocol_header(
    version: ProxyHeader,
    client_ip: IpAddr,
    local_ip: IpAddr,
    client_port: u16,
    local_port: u16,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    match version {
        ProxyHeader::V1 => {
            let is_ipv4 = client_ip.is_ipv4() && local_ip.is_ipv4();
            let proto = if is_ipv4 { "TCP4" } else { "TCP6" };
            let client_str = client_ip.to_string();
            let local_str = local_ip.to_string();
            Ok(
                format!("PROXY {proto} {client_str} {local_str} {client_port} {local_port}\r\n")
                    .into_bytes(),
            )
        }
        ProxyHeader::V2 => {
            let is_ipv4 = client_ip.is_ipv4() && local_ip.is_ipv4();
            let addresses = if is_ipv4 {
                let client_v4 = match client_ip {
                    IpAddr::V4(addr) => addr,
                    _ => return Err("Client IP is not IPv4".into()),
                };
                let local_v4 = match local_ip {
                    IpAddr::V4(addr) => addr,
                    _ => return Err("Local IP is not IPv4".into()),
                };
                ppp::v2::Addresses::IPv4(ppp::v2::IPv4::new(
                    client_v4,
                    local_v4,
                    client_port,
                    local_port,
                ))
            } else {
                let client_v6 = match client_ip {
                    IpAddr::V6(addr) => addr,
                    _ => return Err("Client IP is not IPv6".into()),
                };
                let local_v6 = match local_ip {
                    IpAddr::V6(addr) => addr,
                    _ => return Err("Local IP is not IPv6".into()),
                };
                ppp::v2::Addresses::IPv6(ppp::v2::IPv6::new(
                    client_v6,
                    local_v6,
                    client_port,
                    local_port,
                ))
            };
            let header = ppp::v2::Builder::with_addresses(
                ppp::v2::Version::Two | ppp::v2::Command::Proxy,
                ppp::v2::Protocol::Stream,
                addresses,
            )
            .build()?;
            Ok(header)
        }
    }
}
