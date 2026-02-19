//! Agent identity — Ed25519 keypair and `did:key` DID.
//!
//! Unlike the node-side [`NodeIdentity`], this type is fully synchronous and
//! has no storage dependency. The *host* (native binary, browser, or other
//! WASM runtime) is responsible for persisting the seed and passing it back
//! on restart. This keeps the crate free of I/O and usable from any
//! environment.
//!
//! # Typical host lifecycle
//!
//! ```text
//! First run:
//!   identity = AgentIdentity::generate()
//!   seed     = identity.seed()           // persist this — 32 raw bytes
//!   did      = identity.did()
//!
//! Subsequent runs:
//!   identity = AgentIdentity::from_seed(&saved_seed)
//!   // same DID as before
//! ```

use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;

/// An agent's cryptographic identity.
///
/// The stable public identity is the `did:key` DID, which is derived
/// entirely from the Ed25519 public key and never changes as long as the
/// same seed is used. Agents can therefore be registered on any number of
/// nodes while maintaining a single, portable identity.
#[derive(Clone)]
pub struct AgentIdentity {
    signing_key: SigningKey,
}

impl AgentIdentity {
    /// Generate a fresh identity using OS randomness.
    pub fn generate() -> Self {
        Self {
            signing_key: SigningKey::generate(&mut OsRng),
        }
    }

    /// Restore an identity from a previously saved 32-byte seed.
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(seed),
        }
    }

    /// The raw 32-byte seed. **Keep this secret.** Pass it to
    /// [`from_seed`] on subsequent runs to restore the same identity.
    pub fn seed(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// The agent's stable `did:key` DID, e.g. `did:key:z6Mk…`.
    ///
    /// This is derived from the public key using the multicodec Ed25519
    /// prefix `[0xed, 0x01]` followed by base58btc encoding with a `z`
    /// multibase prefix.
    pub fn did(&self) -> String {
        did_key_from_verifying_key(self.signing_key.verifying_key())
    }

    /// The public key in multibase format (`z`-prefixed base58btc), as
    /// stored in [`AgentProfile::public_key`] and [`NodeInfo::public_key`].
    pub fn public_key_multibase(&self) -> String {
        multibase_from_verifying_key(self.signing_key.verifying_key())
    }

    /// Sign `message` with the agent's private key. Returns the raw 64-byte
    /// Ed25519 signature. The host can use this for HTTP Signatures or any
    /// other application-layer authentication.
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        use ed25519_dalek::Signer;
        self.signing_key.sign(message).to_bytes().to_vec()
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn multibase_from_verifying_key(vk: VerifyingKey) -> String {
    // Multicodec prefix for Ed25519 public key: 0xed 0x01
    let mut prefixed = vec![0xed, 0x01];
    prefixed.extend_from_slice(vk.as_bytes());
    format!("z{}", bs58::encode(prefixed).into_string())
}

fn did_key_from_verifying_key(vk: VerifyingKey) -> String {
    format!("did:key:{}", multibase_from_verifying_key(vk))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_starts_with_z6mk() {
        let id = AgentIdentity::generate();
        assert!(id.did().starts_with("did:key:z6Mk"), "got: {}", id.did());
    }

    #[test]
    fn stable_from_seed() {
        let id = AgentIdentity::generate();
        let seed = id.seed();
        let restored = AgentIdentity::from_seed(&seed);
        assert_eq!(id.did(), restored.did());
    }

    #[test]
    fn public_key_multibase_z_prefix() {
        let id = AgentIdentity::generate();
        assert!(id.public_key_multibase().starts_with('z'));
    }

    #[test]
    fn sign_returns_64_bytes() {
        let id = AgentIdentity::generate();
        let sig = id.sign(b"hello semanticweft");
        assert_eq!(sig.len(), 64);
    }
}
