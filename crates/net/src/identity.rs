//! This computer's device key and self-signed certificate.

use crate::NetError;
use rcgen::PublicKeyData;
use transom_proto::identity::{fingerprint_of_spki, SPKI_PREFIX};

#[derive(Clone)]
pub struct DeviceIdentity {
    pub key_pkcs8_der: Vec<u8>,
    pub cert_der: Vec<u8>,
    pub fingerprint: [u8; 32],
}

fn store_err<E: std::fmt::Display>(e: E) -> NetError {
    NetError::Store(e.to_string())
}

fn from_key_pair(kp: &rcgen::KeyPair) -> Result<DeviceIdentity, NetError> {
    let spki = kp.subject_public_key_info();
    if !spki.starts_with(&SPKI_PREFIX) {
        return Err(NetError::Store("the device key is not a P-256 key".into()));
    }
    let fingerprint = fingerprint_of_spki(&spki)
        .ok_or_else(|| NetError::Store("the device key has an unexpected encoding".into()))?;
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(store_err)?;
    let cn: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).map_err(store_err)?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, cn);
    let cert = params.self_signed(kp).map_err(store_err)?;
    Ok(DeviceIdentity {
        key_pkcs8_der: kp.serialize_der(),
        cert_der: cert.der().to_vec(),
        fingerprint,
    })
}

impl DeviceIdentity {
    /// A new P-256 key; returns it and its PKCS#8 PEM.
    pub fn generate() -> Result<(DeviceIdentity, String), NetError> {
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(store_err)?;
        let pem = kp.serialize_pem();
        Ok((from_key_pair(&kp)?, pem))
    }

    /// Loads the PEM; Err(Store) unless the algorithm is ECDSA P-256. The certificate is rebuilt
    /// at every load: self-signed, CN = 16 random hex characters.
    pub fn from_pem(pem: &str) -> Result<DeviceIdentity, NetError> {
        let kp = rcgen::KeyPair::from_pem(pem).map_err(store_err)?;
        if kp.algorithm() != &rcgen::PKCS_ECDSA_P256_SHA256 {
            return Err(NetError::Store("the device key is not a P-256 key".into()));
        }
        from_key_pair(&kp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_proto::identity::fingerprint_of_certificate;

    #[test]
    fn pem_roundtrip_keeps_fingerprint() {
        let (id, pem) = DeviceIdentity::generate().unwrap();
        let again = DeviceIdentity::from_pem(&pem).unwrap();
        assert_eq!(id.fingerprint, again.fingerprint);
    }

    #[test]
    fn fingerprint_matches_certificate() {
        let (id, _) = DeviceIdentity::generate().unwrap();
        assert_eq!(
            fingerprint_of_certificate(&id.cert_der),
            Some(id.fingerprint)
        );
    }
}
