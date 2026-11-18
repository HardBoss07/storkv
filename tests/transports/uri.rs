use storkv_client::TransportKind;

#[test]
fn parses_every_supported_scheme() {
    let cases = [
        ("storkv://127.0.0.1:7878", TransportKind::NativeTcp),
        ("tcp://127.0.0.1:7878", TransportKind::NativeTcp),
        ("127.0.0.1:7878", TransportKind::NativeTcp),
        ("unix:///tmp/storkv.sock", TransportKind::NativeUnix),
        ("/tmp/storkv.sock", TransportKind::NativeUnix),
        ("http://127.0.0.1:8080", TransportKind::Http),
        ("https://example.test", TransportKind::Http),
        ("ws://127.0.0.1:8081/v1/ws", TransportKind::WebSocket),
        ("grpc://127.0.0.1:9090", TransportKind::Grpc),
    ];
    for (uri, expected) in cases {
        let (kind, _) = TransportKind::parse_uri(uri).expect("parse uri");
        assert_eq!(kind, expected, "uri {uri}");
    }
}

#[test]
fn rejects_unsupported_or_empty_uris() {
    assert!(TransportKind::parse_uri("").is_err());
    assert!(TransportKind::parse_uri("ftp://example.test").is_err());
    assert!(TransportKind::parse_uri("storkv://").is_err());
}

#[test]
fn unix_uri_keeps_absolute_path() {
    let (_, target) = TransportKind::parse_uri("unix:///var/run/storkv.sock").expect("parse");
    assert_eq!(target, "/var/run/storkv.sock");
}
