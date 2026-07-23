use reconciler::{
    CoverageError, CoverageLedger, CoverageState, LineageGraph, LineageNode, LineageNodeKind,
};

#[test]
fn recovery_requires_evidence_and_never_erases_the_gap() {
    let mut ledger = CoverageLedger::new();
    ledger
        .open_gap(
            "bitcoin",
            "mainnet",
            "blocks",
            "observer-a",
            100,
            "zmq_gap",
            1,
        )
        .expect("open");
    assert_eq!(
        ledger.current_state("bitcoin", "mainnet", "blocks", "observer-a"),
        Some(CoverageState::KnownIncomplete)
    );
    assert!(matches!(
        ledger.close_gap(
            "bitcoin",
            "mainnet",
            "blocks",
            "observer-a",
            110,
            std::iter::empty::<&str>(),
            2,
        ),
        Err(CoverageError::MissingRecoveryEvidence)
    ));

    ledger
        .close_gap(
            "bitcoin",
            "mainnet",
            "blocks",
            "observer-a",
            110,
            ["observation-101", "manifest-100-110"],
            2,
        )
        .expect("close");
    assert_eq!(
        ledger.current_state("bitcoin", "mainnet", "blocks", "observer-a"),
        Some(CoverageState::Recovered)
    );
    assert_eq!(ledger.revisions().len(), 2);
    assert_eq!(ledger.revisions()[0].range_start, 100);
    assert_eq!(ledger.revisions()[1].range_end, Some(110));
}

#[test]
fn gaps_are_source_specific_and_revisions_are_monotonic() {
    let mut ledger = CoverageLedger::new();
    ledger
        .open_gap(
            "bitcoin",
            "mainnet",
            "mempool",
            "observer-a",
            10,
            "disconnect",
            5,
        )
        .expect("observer a");
    assert_eq!(
        ledger.current_state("bitcoin", "mainnet", "mempool", "observer-b"),
        None
    );
    assert!(matches!(
        ledger.open_gap(
            "bitcoin",
            "mainnet",
            "mempool",
            "observer-a",
            11,
            "second",
            5,
        ),
        Err(CoverageError::NonMonotonicRevision { .. })
    ));
}

#[test]
fn lineage_is_bounded_and_requires_all_native_layers() {
    let mut graph = LineageGraph::new();
    graph
        .insert(LineageNode::new(
            "obs-1",
            LineageNodeKind::Observation,
            std::iter::empty::<&str>(),
        ))
        .expect("observation");
    graph
        .insert(LineageNode::new(
            "manifest-1",
            LineageNodeKind::ArchiveManifest,
            ["obs-1"],
        ))
        .expect("manifest");
    graph
        .insert(LineageNode::new(
            "fact-1",
            LineageNodeKind::Fact,
            ["manifest-1"],
        ))
        .expect("fact");

    let trace = graph.trace_to_observations("fact-1", 8, 16).expect("trace");
    assert_eq!(
        trace
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        ["fact-1", "manifest-1", "obs-1"]
    );
    assert!(matches!(
        graph.trace_to_observations("fact-1", 1, 16),
        Err(CoverageError::LineageDepthExceeded)
    ));
}
