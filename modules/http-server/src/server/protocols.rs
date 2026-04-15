#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HttpProtocols {
    pub http1: bool,
    pub http2: bool,
}

impl HttpProtocols {
    pub const fn empty() -> Self {
        Self {
            http1: false,
            http2: false,
        }
    }

    pub const fn supports_http1(self) -> bool {
        self.http1
    }

    #[allow(dead_code)]
    pub const fn supports_http2(self) -> bool {
        self.http2
    }

    pub fn alpn_protocols(self) -> Vec<Vec<u8>> {
        let mut protocols = Vec::new();
        if self.http2 {
            protocols.push(b"h2".to_vec());
        }
        if self.http1 {
            protocols.push(b"http/1.1".to_vec());
            protocols.push(b"http/1.0".to_vec());
        }
        protocols
    }
}

impl Default for HttpProtocols {
    fn default() -> Self {
        Self {
            http1: true,
            http2: true,
        }
    }
}
