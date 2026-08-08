#![allow(rustdoc::invalid_html_tags)]
#![allow(clippy::all)]
#![allow(dead_code)]

pub mod opentelemetry {
    pub mod proto {
        pub mod common {
            pub mod v1 {
                #[cfg(test)]
                include!(concat!(
                    env!("OUT_DIR"),
                    "/with_server/opentelemetry.proto.common.v1.rs"
                ));
                #[cfg(not(test))]
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.common.v1.rs"
                ));
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.common.v1.serde.rs"
                ));
            }
        }
        pub mod resource {
            pub mod v1 {
                #[cfg(test)]
                include!(concat!(
                    env!("OUT_DIR"),
                    "/with_server/opentelemetry.proto.resource.v1.rs"
                ));
                #[cfg(not(test))]
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.resource.v1.rs"
                ));
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.resource.v1.serde.rs"
                ));
            }
        }
        pub mod logs {
            pub mod v1 {
                #[cfg(test)]
                include!(concat!(
                    env!("OUT_DIR"),
                    "/with_server/opentelemetry.proto.logs.v1.rs"
                ));
                #[cfg(not(test))]
                include!(concat!(env!("OUT_DIR"), "/opentelemetry.proto.logs.v1.rs"));
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.logs.v1.serde.rs"
                ));
            }
        }
        pub mod metrics {
            pub mod v1 {
                #[cfg(test)]
                include!(concat!(
                    env!("OUT_DIR"),
                    "/with_server/opentelemetry.proto.metrics.v1.rs"
                ));
                #[cfg(not(test))]
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.metrics.v1.rs"
                ));
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.metrics.v1.serde.rs"
                ));
            }
        }
        pub mod trace {
            pub mod v1 {
                #[cfg(test)]
                include!(concat!(
                    env!("OUT_DIR"),
                    "/with_server/opentelemetry.proto.trace.v1.rs"
                ));
                #[cfg(not(test))]
                include!(concat!(env!("OUT_DIR"), "/opentelemetry.proto.trace.v1.rs"));
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.trace.v1.serde.rs"
                ));
            }
        }
        pub mod collector {
            pub mod logs {
                pub mod v1 {
                    #[cfg(test)]
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/with_server/opentelemetry.proto.collector.logs.v1.rs"
                    ));
                    #[cfg(not(test))]
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/opentelemetry.proto.collector.logs.v1.rs"
                    ));
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/opentelemetry.proto.collector.logs.v1.serde.rs"
                    ));
                }
            }
            pub mod metrics {
                pub mod v1 {
                    #[cfg(test)]
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/with_server/opentelemetry.proto.collector.metrics.v1.rs"
                    ));
                    #[cfg(not(test))]
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/opentelemetry.proto.collector.metrics.v1.rs"
                    ));
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/opentelemetry.proto.collector.metrics.v1.serde.rs"
                    ));
                }
            }
            pub mod trace {
                pub mod v1 {
                    #[cfg(test)]
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/with_server/opentelemetry.proto.collector.trace.v1.rs"
                    ));
                    #[cfg(not(test))]
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/opentelemetry.proto.collector.trace.v1.rs"
                    ));
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/opentelemetry.proto.collector.trace.v1.serde.rs"
                    ));
                }
            }
        }
    }
}
