//! Ed25519 signing and verification for [`SemanticUnit`] proofs.
//!
//! Implements ADR-0002: JCS canonicalization (RFC 8785) over the unit with
//! `proof` removed, then Ed25519 sign/verify with base58btc (`z`-prefix)
//! encoding of the signature.

use chrono::Utc;
use ed25519_dalek::{Signer, Verifier};
use thiserror::Error;

use crate::types::{Proof, SemanticUnit};

/// Errors returned by [`sign_unit`].
#[derive(Debug, Error)]
pub enum SigningError {
    #[error("unit already has a proof; remove it before re-signing")]
    AlreadySigned,
    #[error("canonicalization failed: {0}")]
    Canonicalization(String),
}

/// Errors returned by [`verify_proof`].
#[derive(Debug, Error, PartialEq)]
pub enum ProofError {
    #[error("unit has no proof")]
    ProofMissing,
    #[error("proof method is not a valid did:key: {0}")]
    InvalidMethod(String),
    #[error("signature decoding failed: {0}")]
    DecodingFailed(String),
    #[error("signature verification failed")]
    VerificationFailed,
    #[error("canonicalization failed: {0}")]
    Canonicalization(String),
}

/// Sign a unit in place.
///
/// Sets `unit.proof` to a fresh [`Proof`] using the provided Ed25519 key and
/// DID. The `method` field of the proof is `"{did}#{did}"` (self-referencing
/// DID URL, per ADR-0002).
///
/// # Errors
///
/// Returns [`SigningError::AlreadySigned`] if the unit already has a proof.
/// Returns [`SigningError::Canonicalization`] if JCS serialization fails.
pub fn sign_unit(
    unit: &mut SemanticUnit,
    signing_key: &ed25519_dalek::SigningKey,
    did: &str,
) -> Result<(), SigningError> {
    if unit.proof.is_some() {
        return Err(SigningError::AlreadySigned);
    }

    let payload = canonical_payload(unit).map_err(SigningError::Canonicalization)?;
    let signature = signing_key.sign(&payload);
    let sig_bytes = signature.to_bytes();
    let encoded = format!("z{}", bs58::encode(sig_bytes).into_string());

    unit.proof = Some(Proof {
        method: format!("{did}#{did}"),
        created: Utc::now().to_rfc3339(),
        value: encoded,
    });

    Ok(())
}

/// Verify the proof attached to a unit.
///
/// Resolves the signing key from `proof.method` using `did:key` decoding
/// (no network calls). Canonicalizes the unit without its proof field and
/// verifies the Ed25519 signature.
///
/// # Errors
///
/// - [`ProofError::ProofMissing`] — unit has no proof.
/// - [`ProofError::InvalidMethod`] — `proof.method` is not a valid `did:key`.
/// - [`ProofError::DecodingFailed`] — signature bytes cannot be decoded.
/// - [`ProofError::VerificationFailed`] — signature is cryptographically invalid.
pub fn verify_proof(unit: &SemanticUnit) -> Result<(), ProofError> {
    let proof = unit.proof.as_ref().ok_or(ProofError::ProofMissing)?;

    // Decode the verifying key from proof.method.
    // proof.method looks like "did:key:z6Mk...#did:key:z6Mk..." or "did:key:z6Mk...#z6Mk..."
    // We need the part after "did:key:" and before "#" (or the whole thing if no "#").
    let method_did = proof.method.split('#').next().unwrap_or(&proof.method);
    let multibase = method_did
        .strip_prefix("did:key:")
        .ok_or_else(|| ProofError::InvalidMethod(format!("not a did:key: {}", proof.method)))?;

    // multibase starts with 'z' (base58btc). Strip 'z', decode, strip [0xed, 0x01] multicodec prefix.
    let multibase_data = multibase
        .strip_prefix('z')
        .ok_or_else(|| ProofError::InvalidMethod("multibase must start with 'z'".into()))?;

    let decoded = bs58::decode(multibase_data)
        .into_vec()
        .map_err(|e| ProofError::InvalidMethod(format!("base58 decode failed: {e}")))?;

    if decoded.len() < 2 || decoded[0] != 0xed || decoded[1] != 0x01 {
        return Err(ProofError::InvalidMethod(
            "missing ed25519 multicodec prefix [0xed, 0x01]".into(),
        ));
    }

    let key_bytes: [u8; 32] = decoded[2..]
        .try_into()
        .map_err(|_| ProofError::InvalidMethod("key must be 32 bytes".into()))?;

    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes)
        .map_err(|e| ProofError::InvalidMethod(format!("invalid Ed25519 key: {e}")))?;

    // Canonicalize the unit without proof.
    let payload =
        canonical_payload(unit).map_err(ProofError::Canonicalization)?;

    // Decode signature from proof.value (strip 'z', bs58-decode).
    let sig_data = proof
        .value
        .strip_prefix('z')
        .ok_or_else(|| ProofError::DecodingFailed("value must start with 'z'".into()))?;

    let sig_bytes_vec = bs58::decode(sig_data)
        .into_vec()
        .map_err(|e| ProofError::DecodingFailed(format!("bs58 decode failed: {e}")))?;

    let sig_array: [u8; 64] = sig_bytes_vec
        .try_into()
        .map_err(|_| ProofError::DecodingFailed("signature must be exactly 64 bytes".into()))?;

    let signature = ed25519_dalek::Signature::from_bytes(&sig_array);

    verifying_key
        .verify(&payload, &signature)
        .map_err(|_| ProofError::VerificationFailed)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Produce the JCS canonical bytes for a unit with `proof` removed.
fn canonical_payload(unit: &SemanticUnit) -> Result<Vec<u8>, String> {
    // Clone and remove proof before serialising.
    let mut without_proof = unit.clone();
    without_proof.proof = None;

    // Serialise to serde_json::Value first, then JCS-canonicalize.
    let value = serde_json::to_value(&without_proof)
        .map_err(|e| format!("serialization failed: {e}"))?;
    serde_jcs::to_vec(&value).map_err(|e| format!("JCS canonicalization failed: {e}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SemanticUnit, UnitType};
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn test_unit() -> SemanticUnit {
        SemanticUnit::new(
            UnitType::Assertion,
            "The boiling point of water at sea level is 100°C.",
            "did:key:z6MkTest",
        )
    }

    fn test_key() -> (SigningKey, String) {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let pub_bytes = verifying_key.to_bytes();
        let mut multicodec = vec![0xed_u8, 0x01];
        multicodec.extend_from_slice(&pub_bytes);
        let did = format!("did:key:z{}", bs58::encode(&multicodec).into_string());
        (signing_key, did)
    }

    #[test]
    fn sign_then_verify_succeeds() {
        let (signing_key, did) = test_key();
        let mut unit = test_unit();
        sign_unit(&mut unit, &signing_key, &did).expect("signing should succeed");
        assert!(unit.proof.is_some());
        verify_proof(&unit).expect("verification should succeed");
    }

    #[test]
    fn already_signed_returns_error() {
        let (signing_key, did) = test_key();
        let mut unit = test_unit();
        sign_unit(&mut unit, &signing_key, &did).unwrap();
        let result = sign_unit(&mut unit, &signing_key, &did);
        assert!(matches!(result, Err(SigningError::AlreadySigned)));
    }

    #[test]
    fn tamper_content_then_verify_fails() {
        let (signing_key, did) = test_key();
        let mut unit = test_unit();
        sign_unit(&mut unit, &signing_key, &did).unwrap();
        unit.content = "tampered content".into();
        let result = verify_proof(&unit);
        assert!(matches!(result, Err(ProofError::VerificationFailed)));
    }

    #[test]
    fn tamper_signature_then_verify_fails() {
        let (signing_key, did) = test_key();
        let mut unit = test_unit();
        sign_unit(&mut unit, &signing_key, &did).unwrap();
        if let Some(ref mut proof) = unit.proof {
            // Replace last character of value to corrupt it
            let mut corrupted = proof.value.clone();
            let last = corrupted.pop().unwrap_or('A');
            corrupted.push(if last == 'A' { 'B' } else { 'A' });
            proof.value = corrupted;
        }
        let result = verify_proof(&unit);
        // Could be DecodingFailed or VerificationFailed depending on corruption
        assert!(matches!(
            result,
            Err(ProofError::VerificationFailed) | Err(ProofError::DecodingFailed(_))
        ));
    }

    #[test]
    fn missing_proof_returns_proof_missing() {
        let unit = test_unit();
        assert_eq!(verify_proof(&unit), Err(ProofError::ProofMissing));
    }
}
