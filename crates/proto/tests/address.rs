use transom_proto::address::*;

fn ok(s: &str) -> ParsedAddress {
    parse_address(s).unwrap()
}

#[test]
fn accepted() {
    let a = ok("100.101.102.103");
    assert_eq!((a.kind, a.port), (AddressKind::Ipv4, None));
    assert_eq!(ok("100.101.102.103:7455").port, Some(7455));
    let n = ok(" mac.local ");
    assert_eq!((n.kind, n.host.as_str()), (AddressKind::Name, "mac.local"));
    let v = ok("fd7a::1:7455");
    assert_eq!(
        (v.kind, v.host.as_str(), v.port),
        (AddressKind::Ipv6, "fd7a::1:7455", None)
    );
    let v = ok("[fd7a::1]:7455");
    assert_eq!((v.kind, v.port), (AddressKind::Ipv6, Some(7455)));
    let m = ok("[::ffff:1.2.3.4]");
    assert_eq!((m.kind, m.host.as_str()), (AddressKind::Ipv4, "1.2.3.4"));
    assert_eq!(ok("localhost").kind, AddressKind::Name);
    assert_eq!(ok("mac.local.").kind, AddressKind::Name);
}

#[test]
fn refused() {
    let cases = [
        ("1.2.3", AddressError::Ipv4),
        ("0x64.0x65.1.2", AddressError::Ipv4),
        ("01.2.3.4", AddressError::Ipv4),
        ("256.1.1.1", AddressError::Ipv4),
        ("100.1", AddressError::Ipv4),
        ("fe80::1%en0", AddressError::Zone),
        ("sill://pair?v=1", AddressError::PairingLink),
        ("a b", AddressError::Malformed),
        ("", AddressError::Empty),
        ("host:0", AddressError::Port),
        ("host:65536", AddressError::Port),
        ("-bad.com", AddressError::Name),
        ("[1.2.3.4]", AddressError::Ipv6),
        ("[fd7a::1]x", AddressError::Malformed),
    ];
    for (input, err) in cases {
        assert_eq!(parse_address(input), Err(err), "{input:?}");
    }
}

#[test]
fn text_roundtrips() {
    let a = ok("[fd7a::1]:7455");
    assert_eq!(a.text(), "[fd7a::1]:7455");
    assert_eq!(ok(&a.text()), a);
    assert_eq!(ok("host:7455").text(), "host:7455");
}
