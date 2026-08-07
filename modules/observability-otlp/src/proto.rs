#![allow(rustdoc::invalid_html_tags)]

pub mod opentelemetry {
    pub mod proto {
        pub mod common {
            #[allow(clippy::all)]
            pub mod v1 {
                tonic::include_proto!("opentelemetry.proto.common.v1");
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.common.v1.serde.rs"
                ));
            }
        }
        pub mod resource {
            #[allow(clippy::all)]
            pub mod v1 {
                tonic::include_proto!("opentelemetry.proto.resource.v1");
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.resource.v1.serde.rs"
                ));
            }
        }
        pub mod logs {
            #[allow(clippy::all)]
            pub mod v1 {
                tonic::include_proto!("opentelemetry.proto.logs.v1");
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.logs.v1.serde.rs"
                ));
            }
        }
        pub mod metrics {
            #[allow(clippy::all)]
            pub mod v1 {
                tonic::include_proto!("opentelemetry.proto.metrics.v1");
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.metrics.v1.serde.rs"
                ));
            }
        }
        pub mod trace {
            #[allow(clippy::all)]
            pub mod v1 {
                tonic::include_proto!("opentelemetry.proto.trace.v1");
                include!(concat!(
                    env!("OUT_DIR"),
                    "/opentelemetry.proto.trace.v1.serde.rs"
                ));
            }
        }
        pub mod collector {
            pub mod logs {
                #[allow(clippy::all)]
                pub mod v1 {
                    tonic::include_proto!("opentelemetry.proto.collector.logs.v1");
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/opentelemetry.proto.collector.logs.v1.serde.rs"
                    ));
                }
            }
            pub mod metrics {
                #[allow(clippy::all)]
                pub mod v1 {
                    tonic::include_proto!("opentelemetry.proto.collector.metrics.v1");
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/opentelemetry.proto.collector.metrics.v1.serde.rs"
                    ));
                }
            }
            pub mod trace {
                #[allow(clippy::all)]
                pub mod v1 {
                    tonic::include_proto!("opentelemetry.proto.collector.trace.v1");
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/opentelemetry.proto.collector.trace.v1.serde.rs"
                    ));
                }
            }
        }
    }
}
