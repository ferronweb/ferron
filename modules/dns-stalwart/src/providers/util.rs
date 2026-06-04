use std::collections::HashMap;

use ferron_dns::DnsContext;

pub fn required_string(ctx: &DnsContext, key: &str, provider: &str) -> anyhow::Result<String> {
    ctx.config
        .get_value(key)
        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid `{key}` for '{provider}' DNS provider"))
}

pub fn opt_string(ctx: &DnsContext, key: &str) -> Option<String> {
    ctx.config
        .get_value(key)
        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
}

pub fn opt_bool(ctx: &DnsContext, key: &str) -> Option<bool> {
    ctx.config.get_value(key).and_then(|v| v.as_boolean())
}

#[macro_export]
macro_rules! dns_provider {
    ($struct:ident, $name:literal, $key:literal, $updater:ident, $ttl:expr) => {
        pub struct $struct;

        impl ::ferron_core::providers::Provider<::ferron_dns::DnsContext<'static>> for $struct {
            fn name(&self) -> &'static str {
                $name
            }

            fn execute(
                &self,
                ctx: &mut ::ferron_dns::DnsContext,
            ) -> ::std::result::Result<(), Box<dyn ::std::error::Error>> {
                let val = $crate::providers::util::required_string(ctx, $key, $name)?;
                ctx.client = ::std::option::Option::Some(::std::sync::Arc::new(
                    $crate::client::DnsStalwartClient::new(
                        ::dns_update::DnsUpdater::$updater(&val, ::std::option::Option::None)?,
                        $ttl,
                    ),
                ));
                ::std::result::Result::Ok(())
            }
        }
    };
}

#[macro_export]
macro_rules! register_providers {
    ($registry:ident, $($mod:ident => $provider:ident),+ $(,)?) => {
        $registry $(
            .with_provider::<::ferron_dns::DnsContext<'static>, _>(
                || ::std::sync::Arc::new($mod::$provider)
            )
        )+
    };
}

#[macro_export]
macro_rules! register_simple_providers {
    ($registry:ident, $($provider:ident),+ $(,)?) => {
        $registry $(
            .with_provider::<::ferron_dns::DnsContext<'static>, _>(
                || ::std::sync::Arc::new(simple::$provider)
            )
        )+
    };
}
