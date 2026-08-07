//! OTLP wire transports: encoding to OTLP JSON/Protobuf and sending via gRPC
//! (tonic) or HTTP (hyper).

pub mod client;
pub mod grpc;
pub mod http;
pub mod json;
