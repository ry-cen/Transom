use p256::ecdsa::SigningKey;
use sha2::{Digest, Sha256};
use std::time::Instant;
use transom_proto::identity::*;
use transom_proto::messages::{MacInfo, SignedMacInfo};
use transom_proto::pairing::*;

fn sha(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}

fn point_of(secret: [u8; 32]) -> Vec<u8> {
    let sk = SigningKey::from_slice(&secret).unwrap();
    sk.verifying_key().to_sec1_point(false).as_bytes().to_vec()
}

#[test]
fn mac_id_example() {
    assert_eq!(mac_id(&sha("example")), "A3C5HR4RBV67YR21");
    assert_eq!(mac_id(&[0u8; 9]), "");
    assert!(is_well_formed_mac_id("A3C5HR4RBV67YR21"));
    assert!(!is_well_formed_mac_id("a3c5hr4rbv67yr21"));
}

#[test]
fn code_key_vector() {
    assert_eq!(
        hex::encode(sha("mac")),
        "348a629f5ceed032c3e8706ec47d9bfafb00fb4250b018dd965435ca50cb836e"
    );
    assert_eq!(
        hex::encode(sha("device")),
        "263a4dbe41488fb87214b0032339dbb9f0c8da14c16dfcf13084bf3c2552eca5"
    );
    let t = Instant::now();
    let k = code_key("482913557208", &sha("mac")).unwrap();
    assert!(
        t.elapsed().as_secs_f64() < 2.0,
        "code_key too slow: {:?}",
        t.elapsed()
    );
    assert_eq!(
        hex::encode(k),
        "e9baf879a1cc4ac8b21554df480b6c1621671319107c91e7cb05d154f649d31d"
    );
    assert_eq!(code_key("12345", &sha("mac")), None);
    assert_eq!(code_key("48291355720x", &sha("mac")), None);
}

#[test]
fn code_proofs() {
    let (mac, dev) = (sha("mac"), sha("device"));
    let k = code_key("482913557208", &mac).unwrap();
    assert_eq!(
        b64url_encode(&device_proof(&k, &dev, &mac)),
        "dk7PZj4vHsAxj--1u9Vg7ehSn8a1tWc0RCG46YpFVO8"
    );
    assert_eq!(
        b64url_encode(&mac_proof(&k, &mac, &dev)),
        "6BmGWC9XT8J8OY8phhq9bASgoSnoWpRxBeqZtvRrWmk"
    );
}

#[test]
fn link_proofs() {
    let (mac, dev) = (sha("mac"), sha("device"));
    let s = [0x22u8; 16];
    assert_eq!(
        b64url_encode(&device_proof(&s, &dev, &mac)),
        "AyZAJqNTU5nSKEaHx6RZOf0K5dOeygR_8JSrCV_gNOo"
    );
    assert_eq!(
        b64url_encode(&mac_proof(&s, &mac, &dev)),
        "XAX-uZPsq_Ax8fvI_sV7Ex0a-ROi19rI-FW-ykPh9TE"
    );
}

#[test]
fn verify_mac_proof_roundtrip() {
    let (mac, dev) = (sha("mac"), sha("device"));
    let k = [0x22u8; 16];
    let mut p = mac_proof(&k, &mac, &dev);
    assert!(verify_mac_proof(&p, &k, &mac, &dev));
    let d = device_proof(&k, &dev, &mac);
    assert!(verify_device_proof(&d, &k, &dev, &mac));
    assert!(!verify_device_proof(&p, &k, &dev, &mac));
    p[0] ^= 1;
    assert!(!verify_mac_proof(&p, &k, &mac, &dev));
    assert!(!verify_mac_proof(&p[..31], &k, &mac, &dev));
}

#[test]
fn recognition_tag_vector() {
    let t = recognition_tag(&[0x11; 32], [0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6]);
    assert_eq!(t, "oaKjpKWmL4V47Ctm");
}

#[test]
fn tag_matches_roundtrip() {
    let key = [0x11u8; 32];
    let t = recognition_tag(&key, [1, 2, 3, 4, 5, 6]);
    assert!(tag_matches(&t, &key));
    assert!(!tag_matches(&t, &[0x12u8; 32]));
    assert!(!tag_matches("short", &key));
}

#[test]
fn check_code_cases() {
    assert_eq!(check_code("4829 1355 7208"), Ok("482913557208".to_string()));
    assert_eq!(check_code("482913557209"), Err(CodeProblem::Typo));
    assert_eq!(check_code("12345"), Err(CodeProblem::Length));
    assert_eq!(group_code("482913557208"), "4829 1355 7208");
}

#[test]
fn damm_generate_valid() {
    let eleven = [4u8, 8, 2, 9, 1, 3, 5, 5, 7, 2, 0];
    let mut digits = eleven.to_vec();
    digits.push(damm_check_digit(&eleven));
    let code: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
    assert_eq!(check_code(&code), Ok(code.clone()));
    assert_eq!(damm_check_digit(&[10]), 0);
}

#[test]
fn home_door_cases() {
    assert_eq!(home_door(None), HomeDoor::Plain);
    assert_eq!(home_door(Some(None)), HomeDoor::PairingRequired);
    assert_eq!(home_door(Some(Some(b"0"))), HomeDoor::Open);
    assert_eq!(home_door(Some(Some(b"1"))), HomeDoor::PairingRequired);
    assert_eq!(home_door(Some(Some(b""))), HomeDoor::PairingRequired);
    assert_eq!(home_door(Some(Some(b"x"))), HomeDoor::PairingRequired);
}

fn sample_info(id: String) -> MacInfo {
    MacInfo {
        v: 1,
        mac_id: id,
        name: "Test Mac".into(),
        issued_at: 1.5,
        remote_access: true,
        remote_port: 7455,
        internet: false,
        addresses: vec![],
    }
}

#[test]
fn signed_mac_info_verifies() {
    let secret = [1u8; 32];
    let point = point_of(secret);
    let fp = fingerprint_of_point(&point);
    let info = sample_info(mac_id(&fp));
    let signed = SignedMacInfo::sign_for_tests(&info, &secret).unwrap();
    assert_eq!(signed.verified(), Some((info, fp)));
}

#[test]
fn signed_mac_info_tampered() {
    let secret = [1u8; 32];
    let fp = fingerprint_of_point(&point_of(secret));
    let info = sample_info(mac_id(&fp));
    let signed = SignedMacInfo::sign_for_tests(&info, &secret).unwrap();
    let mut bad = signed.clone();
    bad.info = format!("{}A", bad.info);
    assert_eq!(bad.verified(), None);
    let mut bad = signed;
    bad.info = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(b"{}")
    };
    assert_eq!(bad.verified(), None);
    let wrong = sample_info("0000000000000000".into());
    let signed = SignedMacInfo::sign_for_tests(&wrong, &secret).unwrap();
    assert_eq!(signed.verified(), None);
}

#[test]
fn b64url_canonical() {
    assert_eq!(b64url_decode("AA"), Some(vec![0]));
    assert_eq!(b64url_decode("AB"), None);
    assert_eq!(b64url_decode("AA=="), None);
    assert_eq!(b64url_decode("A"), None);
    assert_eq!(b64url_decode("+/"), None);
}
