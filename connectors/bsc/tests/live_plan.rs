use bsc_connector::{BscNodeConfig, BscNodeKind};
use serde_json::Value;
use source_runtime::HttpMethod;

#[test]
fn official_bsc_plan_probes_client_chain_health_and_native_head_states() {
    let config = BscNodeConfig::new(
        "bsc-eu-1",
        "http://127.0.0.1:8545",
        56,
        BscNodeKind::OfficialBsc,
    )
    .expect("config");

    let plan = config.http_poll_plan().expect("HTTP plan");

    assert_eq!(plan.len(), 6);
    assert_rpc(&plan[0], "web3_clientVersion", &[]);
    assert_rpc(&plan[1], "eth_chainId", &[]);
    assert_rpc(&plan[2], "eth_health", &[]);
    assert_rpc(
        &plan[3],
        "eth_getBlockByNumber",
        &[Value::String("latest".to_owned()), Value::Bool(false)],
    );
    assert_rpc(
        &plan[4],
        "eth_getBlockByNumber",
        &[Value::String("safe".to_owned()), Value::Bool(false)],
    );
    assert_rpc(
        &plan[5],
        "eth_getBlockByNumber",
        &[Value::String("finalized".to_owned()), Value::Bool(false)],
    );
    assert_eq!(
        plan[5].source_message_type(),
        "eth_getBlockByNumber.finalized"
    );
}

#[test]
fn websocket_endpoint_is_not_silently_reinterpreted_as_http() {
    let config = BscNodeConfig::new(
        "bsc-eu-1",
        "ws://127.0.0.1:8546",
        56,
        BscNodeKind::OfficialBsc,
    )
    .expect("config");
    assert!(config.http_poll_plan().is_err());
}

fn assert_rpc(request: &source_runtime::HttpRequestSpec, method: &str, params: &[Value]) {
    assert_eq!(request.method(), HttpMethod::Post);
    assert_eq!(request.url(), "http://127.0.0.1:8545/");
    assert_eq!(request.source_channel(), "json_rpc");
    let body: Value = serde_json::from_slice(request.body().expect("JSON-RPC body")).expect("JSON");
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["method"], method);
    assert_eq!(body["params"], Value::Array(params.to_vec()));
}
