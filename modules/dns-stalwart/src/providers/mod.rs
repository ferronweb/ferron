mod alidns;
mod autodns;
mod azuredns;
mod baiducloud;
mod bluecatv2;
mod cloudns;
mod constellix;
mod cpanel;
mod dnsimple;
mod dnsmadeeasy;
mod domeneshop;
mod easydns;
mod edgedns;
mod exoscale;
mod glesys;
mod godaddy;
mod googlecloud;
mod huaweicloud;
mod hurricane;
mod ibmcloud;
mod infoblox;
mod inwx;
mod joker;
mod lightsail;
mod luadns;
mod mythicbeasts;
mod namecheap;
mod namedotcom;
mod netcup;
mod nifcloud;
mod oraclecloud;
mod ovh;
mod plesk;
mod porkbun;
mod rfc2136;
mod route53;
mod simple;
mod spaceship;
mod tencentcloud;
mod transip;
mod ultradns;
mod vercel;
mod volcengine;
mod websupport;
mod yandexcloud;

pub(crate) mod util;

pub fn register_providers(
    registry: ferron_core::registry::RegistryBuilder,
) -> ferron_core::registry::RegistryBuilder {
    let registry = crate::register_providers!(registry,
        alidns => AlibabaCloudDnsProvider,
        autodns => AutoDNSProvider,
        azuredns => AzureDnsProvider,
        baiducloud => BaiduCloudDnsProvider,
        bluecatv2 => BlueCatV2DnsProvider,
        cloudns => ClouDNSProvider,
        constellix => ConstellixDnsProvider,
        cpanel => CpanelDnsProvider,
        dnsimple => DnsimpleDnsProvider,
        dnsmadeeasy => DNSMadeEasyDnsProvider,
        domeneshop => DomeneshopDnsProvider,
        easydns => EasyDNSProvider,
        edgedns => EdgeDnsProvider,
        exoscale => ExoscaleDnsProvider,
        glesys => GlesysDnsProvider,
        godaddy => GoDaddyDnsProvider,
        googlecloud => GoogleCloudDnsProvider,
        huaweicloud => HuaweiCloudDnsProvider,
        hurricane => HurricaneProvider,
        ibmcloud => IbmCloudDnsProvider,
        infoblox => InfobloxDnsProvider,
        inwx => InwxDnsProvider,
        joker => JokerDnsProvider,
        lightsail => LightsailDnsProvider,
        luadns => LuaDnsDnsProvider,
        mythicbeasts => MythicBeastsDnsProvider,
        namecheap => NamecheapDnsProvider,
        namedotcom => NameDotComDnsProvider,
        netcup => NetcupDnsProvider,
        nifcloud => NifcloudDnsProvider,
        oraclecloud => OracleCloudDnsProvider,
        ovh => OvhDnsProvider,
        plesk => PleskDnsProvider,
        porkbun => PorkbunDnsProvider,
        rfc2136 => Rfc2136DnsProvider,
        route53 => Route53DnsProvider,
        spaceship => SpaceshipDnsProvider,
        tencentcloud => TencentCloudDnsProvider,
        transip => TransipDnsProvider,
        ultradns => UltraDnsDnsProvider,
        vercel => VercelDnsProvider,
        volcengine => VolcengineDnsProvider,
        websupport => WebSupportDnsProvider,
        yandexcloud => YandexCloudDnsProvider,
    );
    crate::register_simple_providers!(
        registry,
        ArvanCloudDnsProvider,
        BunnyDnsProvider,
        CloudflareDnsProvider,
        DDNSSProvider,
        DesecDnsProvider,
        DigitalOceanDnsProvider,
        DreamHostDnsProvider,
        DuckDNSProvider,
        DynuDnsProvider,
        FreeMyIpDnsProvider,
        GandiV5DnsProvider,
        GcoreDnsProvider,
        HetznerDnsProvider,
        HostingDeDnsProvider,
        HostingerDnsProvider,
        InfomaniakDnsProvider,
        IonosDnsProvider,
        IPv64DnsProvider,
        LinodeDnsProvider,
        NameSiloDnsProvider,
        NetlifyDnsProvider,
        NS1Provider,
        SafeDNSProvider,
        ScalewayDnsProvider,
        VultrDnsProvider
    )
}
