use bitcoin_domain::parse_transaction;
use bitcoin_mempool::{MempoolError, MempoolGraph, ReplacementClassification, ReplacementEvidence};

#[test]
fn conflicts_and_replacements_preserve_evidence_strength() {
    let first = parse_transaction(&fixture("conflict_a.hex")).expect("conflict a");
    let second = parse_transaction(&fixture("conflict_b.hex")).expect("conflict b");
    let mut graph = MempoolGraph::new("observer-a").expect("graph");

    graph
        .add(first.clone(), 100, ReplacementEvidence::None)
        .expect("first");
    graph.remove(first.txid());
    let update = graph
        .add(
            second.clone(),
            200,
            ReplacementEvidence::Direct {
                replaced_txid: first.txid(),
            },
        )
        .expect("replacement");

    assert_eq!(update.conflicts().len(), 1);
    assert_eq!(
        update.replacement_classification(),
        Some(ReplacementClassification::Observed)
    );
    assert_eq!(update.conflicts()[0].source_id(), "observer-a");
    assert_eq!(update.conflicts()[0].txid(), second.txid());
    assert_eq!(update.conflicts()[0].conflicting_txid(), first.txid());

    let mut inferred = MempoolGraph::new("observer-b").expect("graph");
    inferred
        .add(first.clone(), 100, ReplacementEvidence::None)
        .expect("first");
    inferred.remove(first.txid());
    let inferred_update = inferred
        .add(second, 200, ReplacementEvidence::None)
        .expect("conflict");
    assert_eq!(
        inferred_update.replacement_classification(),
        Some(ReplacementClassification::Inferred)
    );
}

#[test]
fn cpfp_package_is_deterministic_and_checked() {
    let parent = parse_transaction(&fixture("cpfp_parent.hex")).expect("parent");
    let child = parse_transaction(&fixture("cpfp_child.hex")).expect("child");
    let mut graph = MempoolGraph::new("observer-a").expect("graph");
    graph
        .add(parent.clone(), 100, ReplacementEvidence::None)
        .expect("parent");
    graph
        .add(child.clone(), 10_000, ReplacementEvidence::None)
        .expect("child");

    let package = graph.package(child.txid()).expect("package");
    let mut expected = vec![parent.txid(), child.txid()];
    expected.sort_unstable();
    assert_eq!(package.member_txids(), expected);
    assert_eq!(package.total_fee_sats(), 10_100);
    assert_eq!(
        package.total_vsize(),
        u64::try_from(parent.virtual_size() + child.virtual_size()).expect("small fixture")
    );
    assert_eq!(
        package.effective_fee_rate().fee_sats(),
        package.total_fee_sats()
    );
    assert_eq!(package.effective_fee_rate().vsize(), package.total_vsize());

    let mut overflow = MempoolGraph::new("observer-a").expect("graph");
    overflow
        .add(parent, u64::MAX, ReplacementEvidence::None)
        .expect("parent");
    overflow
        .add(child.clone(), 1, ReplacementEvidence::None)
        .expect("child");
    assert!(matches!(
        overflow.package(child.txid()),
        Err(MempoolError::FeeOverflow)
    ));
}

fn fixture(name: &str) -> Vec<u8> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/bitcoin/objects")
        .join(name);
    let text = std::fs::read_to_string(root).expect("fixture");
    (0..text.trim().len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
        .collect()
}
