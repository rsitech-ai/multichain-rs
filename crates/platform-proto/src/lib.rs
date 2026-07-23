#![doc = "Generated platform wire contracts and their canonical descriptor."]
#![allow(clippy::doc_markdown)]

/// Bitcoin-native wire contracts.
pub mod bitcoin {
    include!("generated/platform/bitcoin/v1/platform.bitcoin.v1.rs");
}

/// Operational control-plane wire contracts.
pub mod control {
    include!("generated/platform/control/v1/platform.control.v1.rs");
}

/// Normalized fact wire contracts.
pub mod fact {
    include!("generated/platform/fact/v1/platform.fact.v1.rs");
}

/// Exact source-observation wire contracts.
pub mod observation {
    include!("generated/platform/observation/v1/platform.observation.v1.rs");
}

/// Canonical descriptor set used for reflection and compatibility checks.
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!("../../../schemas/protobuf/platform.binpb");
