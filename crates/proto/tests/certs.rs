use transom_proto::identity::*;

const P256: &[u8] = include_bytes!("fixtures/cert-p256.der");
const SPKI: &[u8] = include_bytes!("fixtures/cert-p256.spki.der");
const P384: &[u8] = include_bytes!("fixtures/cert-p384.der");

#[test]
fn spki_of_fixture() {
    assert_eq!(spki_from_certificate(P256), Some(SPKI));
}

#[test]
fn fingerprint_of_fixture() {
    let fp = fingerprint_of_certificate(P256).unwrap();
    let hex: String = fp.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex,
        "8199caa5abd85ba6437bc238da2a3f00b017184313978b33a6c7fae20522c428"
    );
}

#[test]
fn p384_fixture_has_no_fingerprint() {
    assert!(spki_from_certificate(P384).is_some());
    assert_eq!(fingerprint_of_certificate(P384), None);
}

#[test]
fn truncated_certificate_is_none() {
    for n in 0..P256.len() {
        assert_eq!(spki_from_certificate(&P256[..n]), None, "prefix {n}");
        assert_eq!(fingerprint_of_certificate(&P256[..n]), None);
    }
}
