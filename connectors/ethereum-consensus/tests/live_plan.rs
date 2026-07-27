use ethereum_consensus_connector::ConsensusSourceConfig;
use source_runtime::HttpMethod;

#[test]
fn consensus_plan_keeps_head_and_finalized_observations_separate() {
    let config =
        ConsensusSourceConfig::new("lighthouse-eu-1", "http://127.0.0.1:5052").expect("config");

    let plan = config.http_poll_plan().expect("HTTP plan");

    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].method(), HttpMethod::Get);
    assert_eq!(
        plan[0].url(),
        "http://127.0.0.1:5052/eth/v2/beacon/blocks/head"
    );
    assert_eq!(plan[0].source_channel(), "beacon_api");
    assert_eq!(plan[0].source_message_type(), "beacon_block.head");
    assert_eq!(
        plan[1].url(),
        "http://127.0.0.1:5052/eth/v2/beacon/blocks/finalized"
    );
    assert_eq!(plan[1].source_message_type(), "beacon_block.finalized");
}
