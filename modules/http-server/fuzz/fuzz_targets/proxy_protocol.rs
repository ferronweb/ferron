#![no_main]

use libfuzzer_sys::fuzz_target;
use ppp::HeaderResult;
use std::net::{IpAddr, SocketAddr};

fuzz_target!(|data: &[u8]| {
    // Try to parse the PROXY protocol header from arbitrary bytes
    let header = HeaderResult::parse(data);

    match header {
        HeaderResult::V1(Ok(h)) => {
            // Convert addresses to SocketAddr as the real code does
            match h.addresses {
                ppp::v1::Addresses::Tcp4(ip) => {
                    let _client = SocketAddr::new(IpAddr::V4(ip.source_address), ip.source_port);
                    let _server = SocketAddr::new(IpAddr::V4(ip.destination_address), ip.destination_port);
                }
                ppp::v1::Addresses::Tcp6(ip) => {
                    let _client = SocketAddr::new(IpAddr::V6(ip.source_address), ip.source_port);
                    let _server = SocketAddr::new(IpAddr::V6(ip.destination_address), ip.destination_port);
                }
                ppp::v1::Addresses::Unknown => {
                    // Accept unknown as valid parse path
                }
            }
        }
        HeaderResult::V2(Ok(h)) => {
            match h.addresses {
                ppp::v2::Addresses::IPv4(ip) => {
                    let _client = SocketAddr::new(IpAddr::V4(ip.source_address), ip.source_port);
                    let _server = SocketAddr::new(IpAddr::V4(ip.destination_address), ip.destination_port);
                }
                ppp::v2::Addresses::IPv6(ip) => {
                    let _client = SocketAddr::new(IpAddr::V6(ip.source_address), ip.source_port);
                    let _server = SocketAddr::new(IpAddr::V6(ip.destination_address), ip.destination_port);
                }
                ppp::v2::Addresses::Unix(_u) => {
                    // unix addresses not supported in runtime path
                }
                ppp::v2::Addresses::Unspecified => {}
            }
        }
        _ => {
            // Invalid parse branches are expected; do nothing
        }
    }
});
