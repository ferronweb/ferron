#[cfg(feature = "bunny")]
mod bunny;
#[cfg(feature = "cloudflare")]
mod cloudflare;
#[cfg(feature = "desec")]
mod desec;
#[cfg(feature = "digitalocean")]
mod digitalocean;
#[cfg(feature = "ovh")]
mod ovh;
#[cfg(feature = "porkbun")]
mod porkbun;
#[cfg(feature = "rfc2136")]
mod rfc2136;
#[cfg(feature = "route53")]
mod route53;

#[cfg(feature = "bunny")]
pub use bunny::*;
#[cfg(feature = "cloudflare")]
pub use cloudflare::*;
#[cfg(feature = "desec")]
pub use desec::*;
#[cfg(feature = "digitalocean")]
pub use digitalocean::*;
#[cfg(feature = "ovh")]
pub use ovh::*;
#[cfg(feature = "porkbun")]
pub use porkbun::*;
#[cfg(feature = "rfc2136")]
pub use rfc2136::*;
#[cfg(feature = "route53")]
pub use route53::*;
