/// Chain-neutral canonicality state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Canonicality {
    NotApplicable,
    Candidate,
    Canonical,
    NonCanonical,
}

/// Chain-neutral settlement/finality state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Finality {
    NotApplicable,
    Pending,
    Included,
    Safe,
    Finalized,
    Reorged,
}
