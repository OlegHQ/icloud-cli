use icloud_api::session::SessionData;

#[test]
fn session_roundtrips_minimal_json() {
    let raw = r#"{
        "ck_base_url": "https://example/",
        "cookies": [],
        "dsid": "123"
    }"#;
    let s: SessionData = serde_json::from_str(raw).unwrap();
    assert_eq!(s.ck_base_url, "https://example/");
    assert_eq!(s.dsid.as_deref(), Some("123"));
}
