//! HTTP pipeline stages

mod client_ip;
mod https_redirect;

pub use client_ip::ClientIpFromHeaderStage;
pub use https_redirect::HttpsRedirectStage;
