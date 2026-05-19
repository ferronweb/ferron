mod alidns;
mod arvancloud;
mod autodns;
mod azuredns;
mod baiducloud;
mod bluecatv2;
mod bunny;
mod cloudflare;
mod cloudns;
mod constellix;
mod cpanel;
mod ddnss;
mod desec;
mod digitalocean;
mod dnsimple;
mod dnsmadeeasy;
mod domeneshop;
mod dreamhost;
mod duckdns;
mod dynu;
mod easydns;
mod edgedns;
mod exoscale;
mod freemyip;
mod gandiv5;
mod gcore;
mod glesys;
mod godaddy;
mod googlecloud;
mod hetzner;
mod hostingde;
mod hostinger;
mod huaweicloud;
mod hurricane;
mod ibmcloud;
mod infoblox;
mod infomaniak;
mod inwx;
mod ionos;
mod ipv64;
mod joker;
mod lightsail;
mod linode;
mod luadns;
mod mythicbeasts;
mod namecheap;
mod namedotcom;
mod namesilo;
mod netcup;
mod netlify;
mod nifcloud;
mod ns1;
mod oraclecloud;
mod ovh;
mod plesk;
mod porkbun;
mod rfc2136;
mod route53;
mod safedns;
mod scaleway;
mod spaceship;
mod tencentcloud;
mod transip;
mod ultradns;
mod vercel;
mod volcengine;
mod vultr;
mod websupport;
mod yandexcloud;

use std::sync::Arc;

use ferron_dns::DnsContext;

pub fn register_providers(
    registry: ferron_core::registry::RegistryBuilder,
) -> ferron_core::registry::RegistryBuilder {
    registry
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(alidns::AlibabaCloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(arvancloud::ArvanCloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(autodns::AutoDNSProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(azuredns::AzureDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(baiducloud::BaiduCloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(bluecatv2::BlueCatV2DnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(bunny::BunnyDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(cloudflare::CloudflareDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(cloudns::ClouDNSProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(constellix::ConstellixDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(cpanel::CpanelDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(ddnss::DDNSSProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(desec::DesecDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(digitalocean::DigitalOceanDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(dnsimple::DnsimpleDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(dnsmadeeasy::DNSMadeEasyDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(domeneshop::DomeneshopDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(dreamhost::DreamHostDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(duckdns::DuckDNSProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(dynu::DynuDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(easydns::EasyDNSProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(edgedns::EdgeDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(exoscale::ExoscaleDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(freemyip::FreeMyIpDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(gandiv5::GandiV5DnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(gcore::GcoreDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(glesys::GlesysDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(godaddy::GoDaddyDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(googlecloud::GoogleCloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(hetzner::HetznerDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(hostingde::HostingDeDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(huaweicloud::HuaweiCloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(ibmcloud::IbmCloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(hostinger::HostingerDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(hurricane::HurricaneProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(infoblox::InfobloxDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(infomaniak::InfomaniakDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(inwx::InwxDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(ionos::IonosDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(ipv64::IPv64DnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(joker::JokerDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(linode::LinodeDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(lightsail::LightsailDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(luadns::LuaDnsDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(mythicbeasts::MythicBeastsDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(namecheap::NamecheapDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(namedotcom::NameDotComDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(namesilo::NameSiloDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(netcup::NetcupDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(netlify::NetlifyDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(nifcloud::NifcloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(ns1::NS1Provider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(oraclecloud::OracleCloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(ovh::OvhDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(plesk::PleskDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(porkbun::PorkbunDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(rfc2136::Rfc2136DnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(route53::Route53DnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(safedns::SafeDNSProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(scaleway::ScalewayDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(spaceship::SpaceshipDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(tencentcloud::TencentCloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(transip::TransipDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(vercel::VercelDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(vultr::VultrDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(websupport::WebSupportDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(volcengine::VolcengineDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(yandexcloud::YandexCloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(ultradns::UltraDnsDnsProvider))
}
