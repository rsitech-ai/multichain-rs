use bitcoin_domain::Sats;

#[test]
fn api_amounts_are_decimal_strings_and_bounded_by_u64() {
    assert_eq!(
        serde_json::to_string(&Sats::new(2_100_000_000_000_000)).expect("serialize"),
        "\"2100000000000000\""
    );
    assert_eq!(
        serde_json::from_str::<Sats>("\"18446744073709551615\"")
            .expect("u64 maximum")
            .value(),
        u64::MAX
    );
    assert!(serde_json::from_str::<Sats>("18446744073709551615").is_err());
    assert!(serde_json::from_str::<Sats>("\"18446744073709551616\"").is_err());
    assert!(serde_json::from_str::<Sats>("\"-1\"").is_err());
}

#[test]
fn checked_amount_addition_never_wraps() {
    assert_eq!(Sats::new(41).checked_add(Sats::new(1)), Some(Sats::new(42)));
    assert_eq!(Sats::new(u64::MAX).checked_add(Sats::new(1)), None);
}
