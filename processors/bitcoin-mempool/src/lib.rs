#![doc = "Source-qualified Bitcoin mempool state and aggregate views."]

mod aggregate;
mod error;
mod graph;
mod membership;
mod state;

pub use aggregate::{
    AggregateMembership, CoverageQuality, MempoolViewPolicy, aggregate_membership,
};
pub use error::MempoolError;
pub use graph::{
    ConflictEdge, FeeRate, GraphUpdate, MempoolGraph, PackageSnapshot, ReplacementClassification,
    ReplacementEvidence,
};
pub use membership::{MembershipCause, MembershipRevision, MembershipState, RemovalCause};
pub use state::{ObserverHealth, ObserverMempool};

/// Stable component identifier used by health and build metadata.
pub const COMPONENT_NAME: &str = "bitcoin-mempool";
