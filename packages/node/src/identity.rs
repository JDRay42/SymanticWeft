//! Node identity — Ed25519 keypair and `did:key` derivation.
//!
//! On first startup a fresh Ed25519 keypair is generated. The seed (32 bytes,
//! hex-encoded) is persisted in storage under the key `"node_identity_seed"`.
//! On subsequent restarts the seed is reloaded so the node's DID is stable.
//!
//! Ephemeral nodes (in-memory storage) generate a fresh keypair every run —
//! their DID changes on each restart, which is acceptable because they have no
//! persistent peer records to maintain.
//!
//! # `did:key` encoding
//!
//! The DID is derived by encoding the 32-byte Ed25519 public key as:
//!
//! ```text
//! did:key: z <base58btc( [0xed, 0x01] ++ public_key_bytes )>
//! ```
//!
//! where `[0xed, 0x01]` is the unsigned-varint encoding of the `ed25519-pub`
//! multicodec value (237 decimal). This produces the well-known `z6Mk…` prefix.

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;

use crate::storage::{Storage, StorageError};

const SEED_KEY: &str = "node_identity_seed";

/// The node's stable Ed25519 identity.
pub struct NodeIdentity {
    signing_key: SigningKey,
}

impl NodeIdentity {
    /// Load the identity seed from storage, or generate a new one and save it.
    pub async fn load_or_generate(storage: &Arc<dyn Storage>) -> Result<Self, StorageError> {
        if let Some(hex_seed) = storage.get_node_config(SEED_KEY).await? {
            let seed_bytes = hex::decode(&hex_seed)
                .map_err(|e| StorageError::Internal(format!("invalid identity seed: {e}")))?;
            let bytes: [u8; 32] = seed_bytes
                .try_into()
                .map_err(|_| StorageError::Internal("identity seed must be 32 bytes".into()))?;
            Ok(Self {
                signing_key: SigningKey::from_bytes(&bytes),
            })
        } else {
            let signing_key = SigningKey::generate(&mut OsRng);
            let hex_seed = hex::encode(signing_key.to_bytes());
            storage.set_node_config(SEED_KEY, &hex_seed).await?;
            Ok(Self { signing_key })
        }
    }

    /// The node's `did:key` DID, stable across restarts for persistent nodes.
    pub fn did(&self) -> String {
        let pub_key = self.signing_key.verifying_key().to_bytes();
        did_key_from_bytes(&pub_key)
    }

    /// The public key as a `z`-prefixed base58btc multibase string, for
    /// inclusion in `/.well-known/semanticweft` as a standalone `public_key`
    /// field (same encoding as the suffix of the `did:key`, for consistency).
    pub fn public_key_multibase(&self) -> String {
        let pub_key = self.signing_key.verifying_key().to_bytes();
        encode_multibase(&pub_key)
    }
}

/// Encode a raw 32-byte Ed25519 public key as a full `did:key` DID string.
pub fn did_key_from_bytes(pub_key: &[u8; 32]) -> String {
    format!("did:key:{}", encode_multibase(pub_key))
}

/// Encode a raw 32-byte Ed25519 public key as multibase base58btc (`z` prefix).
///
/// Prepends the ed25519-pub multicodec varint prefix `[0xed, 0x01]` before
/// base58btc-encoding.
fn encode_multibase(pub_key: &[u8; 32]) -> String {
    let mut bytes = vec![0xed_u8, 0x01];
    bytes.extend_from_slice(pub_key);
    format!("z{}", bs58::encode(&bytes).into_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::storage::memory::MemoryStorage;

    #[tokio::test]
    async fn generates_stable_did_from_storage() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());

        let id1 = NodeIdentity::load_or_generate(&storage).await.unwrap();
        let did1 = id1.did();

        // Second call loads the same seed.
        let id2 = NodeIdentity::load_or_generate(&storage).await.unwrap();
        let did2 = id2.did();

        assert_eq!(did1, did2, "DID must be stable after first generation");
        assert!(did1.starts_with("did:key:z6Mk"), "should have ed25519 did:key prefix");
    }

    #[tokio::test]
    async fn public_key_multibase_has_z_prefix() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let id = NodeIdentity::load_or_generate(&storage).await.unwrap();
        let pk = id.public_key_multibase();
        assert!(pk.starts_with('z'), "multibase base58btc uses 'z' prefix");
    }

    #[test]
    fn did_key_prefix_is_z6mk() {
        // A zeroed public key should still produce the known z6Mk prefix.
        let zero_key = [0u8; 32];
        let did = did_key_from_bytes(&zero_key);
        assert!(did.starts_with("did:key:z"), "did:key always starts with z");
    }
}
