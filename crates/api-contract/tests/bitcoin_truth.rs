use api_contract::{
    BitcoinOutput, BitcoinTruth, Completeness, MempoolView, PageCursor, ResyncReason,
    StreamControl, TruthError,
};
use bitcoin_domain::Sats;

#[test]
fn bitcoin_amounts_and_truth_metadata_have_stable_json_types() {
    let truth = BitcoinTruth::new(
        "mainnet",
        ["observer-eu-1", "observer-us-1"],
        42,
        "canonical",
        "confirmed",
        840_000,
        Completeness::Complete,
        1_721_000_000_000_000_000,
    )
    .expect("truth");
    let output = BitcoinOutput::new(
        "00".repeat(32),
        1,
        Sats::new(2_100_000_000_000_000),
        "51",
        truth,
    )
    .expect("output");

    let json = serde_json::to_value(output).expect("json");
    assert_eq!(json["value_sats"], "2100000000000000");
    assert_eq!(json["truth"]["revision"], 42);
    assert_eq!(json["truth"]["source_ids"][0], "observer-eu-1");
    assert_eq!(json["truth"]["canonicality"], "canonical");
    assert_eq!(json["truth"]["finality"], "confirmed");
    assert_eq!(json["truth"]["completeness"], "complete");
}

#[test]
fn truth_rejects_missing_sources_zero_revision_and_invalid_status_pairs() {
    assert!(matches!(
        BitcoinTruth::new(
            "mainnet",
            std::iter::empty::<&str>(),
            1,
            "canonical",
            "confirmed",
            1,
            Completeness::Complete,
            1,
        ),
        Err(TruthError::MissingSources)
    ));
    assert!(matches!(
        BitcoinTruth::new(
            "mainnet",
            ["observer-a"],
            0,
            "canonical",
            "confirmed",
            1,
            Completeness::Complete,
            1,
        ),
        Err(TruthError::ZeroRevision)
    ));
    assert!(matches!(
        BitcoinTruth::new(
            "mainnet",
            ["observer-a"],
            1,
            "non_canonical",
            "confirmed",
            1,
            Completeness::Complete,
            1,
        ),
        Err(TruthError::InconsistentBitcoinStatus { .. })
    ));
}

#[test]
fn mempool_views_are_never_implicitly_global() {
    assert!(matches!(
        MempoolView::source(""),
        Err(TruthError::EmptySourceId)
    ));
    assert!(matches!(
        MempoolView::quorum(0, ["observer-a"]),
        Err(TruthError::InvalidQuorum { .. })
    ));
    assert!(matches!(
        MempoolView::quorum(2, ["observer-a"]),
        Err(TruthError::InvalidQuorum { .. })
    ));

    let quorum = MempoolView::quorum(2, ["observer-b", "observer-a"]).expect("quorum");
    let json = serde_json::to_value(quorum).expect("json");
    assert_eq!(json["kind"], "quorum");
    assert_eq!(json["threshold"], 2);
    assert_eq!(json["eligible_source_ids"][0], "observer-a");
}

#[test]
fn pagination_and_stream_resync_are_bounded_and_explicit() {
    let cursor = PageCursor::new("bitcoin_blocks", "0000ff", 19).expect("cursor");
    let encoded = cursor.encode();
    assert_eq!(PageCursor::decode(&encoded).expect("decode"), cursor);
    assert!(PageCursor::decode(&format!("{encoded}00")).is_err());

    let control =
        StreamControl::resync_required("bitcoin.blocks", 100, 120, ResyncReason::SequenceGap)
            .expect("control");
    let json = serde_json::to_value(control).expect("json");
    assert_eq!(json["type"], "resync_required");
    assert_eq!(json["last_delivered_sequence"], 100);
    assert_eq!(json["current_sequence"], 120);
}
