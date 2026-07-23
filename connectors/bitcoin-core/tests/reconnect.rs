use bitcoin_core_connector::session::SourceSession;

#[test]
fn reconnect_closes_prior_session_and_restarts_total_order() {
    let mut prior = SourceSession::with_id("observer-a", [1; 16], 100);
    assert_eq!(prior.allocate().get(), 0);
    assert_eq!(prior.allocate().get(), 1);

    let (mut next, closed) = prior.reconnect([2; 16], 200);
    assert_eq!(closed.source_session_id, vec![1; 16]);
    assert_eq!(closed.end_unix_ns, Some(200));
    assert_eq!(closed.state, "closed");
    assert_eq!(next.id().as_bytes(), &[2; 16]);
    assert_eq!(next.allocate().get(), 0);
}
