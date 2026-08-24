// C library signature is: int inet_aton(const char *cp, struct in_addr *inp);

use std::str::FromStr;
#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "solaris"
))]
unsafe extern "C" {
    fn inet_aton(source: *const u8, ip: *mut libc::in_addr) -> bool;
}

/// Tries to convert string to IP address, including alternative representations.
#[inline]
pub fn convert(source: &str) -> Option<std::net::IpAddr> {
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "solaris"
    ))]
    {
        use std::ffi::CString;

        let mut ip = std::mem::MaybeUninit::<libc::in_addr>::uninit();
        // inet_aton accepts null-terminated C string, not Rust &str, so convert it
        if let Ok(source_c) = CString::new(source) {
            // SAFETY: pointer to in_addr is valid.
            if unsafe { inet_aton(source_c.as_ptr() as *const u8, ip.as_mut_ptr()) } {
                // SAFETY: conversion is successful and in_addr is initialized.
                let ip = unsafe { ip.assume_init() };
                // in_addr.s_addr is in network byte order (big-endian),
                // while Rust's Ipv4Addr is host order.
                let ip_u32 = u32::from_be(ip.s_addr);
                return Some(std::net::Ipv4Addr::from(ip_u32).into());
            }
        }
    }
    #[cfg(windows)]
    {
        // Winsock functions need WSAStartup first...
        let mut wsa_data: windows::Win32::Networking::WinSock::WSADATA = Default::default();
        let _ = unsafe { windows::Win32::Networking::WinSock::WSAStartup(0x202, &mut wsa_data) };

        // There are two variants: WSAStringToAddressA and WSAStringToAddressW.
        // Let's use the "W" variant, since it supports Unicode strings
        //
        // But first, let's convert Rust string to PCWSTR through HSTRING...
        let source_hstring = windows::core::HSTRING::from(source);
        {
            let mut ip: windows::Win32::Networking::WinSock::SOCKADDR_IN = Default::default();
            let mut ip_size =
                std::mem::size_of::<windows::Win32::Networking::WinSock::SOCKADDR_IN>() as i32;
            // SAFETY: SOCKADDR struct pointer is valid and so is the length
            // (SOCKADDR_IN, which is IPv4)
            let result = unsafe {
                windows::Win32::Networking::WinSock::WSAStringToAddressW(
                    &source_hstring,
                    windows::Win32::Networking::WinSock::AF_INET.0 as i32,
                    None,
                    &mut ip as *mut windows::Win32::Networking::WinSock::SOCKADDR_IN
                        as *mut windows::Win32::Networking::WinSock::SOCKADDR,
                    &mut ip_size,
                )
            };
            if result == 0
                && ip_size
                    == std::mem::size_of::<windows::Win32::Networking::WinSock::SOCKADDR_IN>()
                        as i32
            {
                // Similar as libc
                return Some(std::net::Ipv4Addr::from(ip.sin_addr).into());
            }
        }
        {
            // WSAStringtoAddressW also supports IPv6
            let mut ip: windows::Win32::Networking::WinSock::SOCKADDR_IN6 = Default::default();
            let mut ip_size =
                std::mem::size_of::<windows::Win32::Networking::WinSock::SOCKADDR_IN6>() as i32;
            // SAFETY: SOCKADDR struct pointer is valid and so is the length
            // (SOCKADDR_IN6, which is IPv6)
            let result = unsafe {
                windows::Win32::Networking::WinSock::WSAStringToAddressW(
                    &source_hstring,
                    windows::Win32::Networking::WinSock::AF_INET6.0 as i32,
                    None,
                    &mut ip as *mut windows::Win32::Networking::WinSock::SOCKADDR_IN6
                        as *mut windows::Win32::Networking::WinSock::SOCKADDR,
                    &mut ip_size,
                )
            };
            if result == 0
                && ip_size
                    == std::mem::size_of::<windows::Win32::Networking::WinSock::SOCKADDR_IN6>()
                        as i32
            {
                let addr: std::net::IpAddr = std::net::Ipv6Addr::from(ip.sin6_addr).into();
                return Some(addr.to_canonical());
            }
        }
    }
    // Cross-platform fallback that doesn't support alternative representations...
    std::net::IpAddr::from_str(source)
        .map(|ip| ip.to_canonical())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::convert;

    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "solaris"
    ))]
    #[test]
    fn libc_inet_aton_parses_alternative_ip() {
        assert_eq!(convert("0x7f.0.0.1"), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(convert("2130706433"), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(convert("0177.0.0.1"), Some("127.0.0.1".parse().unwrap()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_inet_aton_parses_alternative_ip() {
        assert_eq!(convert("0x7f.0.0.1"), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(convert("2130706433"), Some("127.0.0.1".parse().unwrap()));
        assert_eq!(convert("0177.0.0.1"), Some("127.0.0.1".parse().unwrap()));
    }
}
