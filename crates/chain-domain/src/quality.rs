/// Explicit quality markers propagated with normalized facts.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum QualityFlag {
    Recovered,
    KnownIncomplete,
    SourceDivergence,
    ClockSkew,
    DecoderFallback,
}
