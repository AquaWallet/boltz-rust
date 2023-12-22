use std::str::FromStr;

use lightning_invoice::Bolt11Invoice;
use secp256k1::hashes::{hash160, ripemd160, sha256, Hash};

use crate::{
    e::{ErrorKind, S5Error},
    util::rnd_str,
};

#[derive(Debug, Clone)]
pub struct PreimageStates {
    pub preimage: Option<String>,
    pub preimage_bytes: Option<Vec<u8>>,
    pub sha256: String,
    pub hash160: String,
    pub sha256_bytes: [u8; 32],
    pub hash160_bytes: [u8; 20],
}

impl PreimageStates {
    pub fn new() -> PreimageStates {
        let preimage = rnd_str();
        let sha256 = sha256::Hash::hash(&hex::decode(preimage.clone()).unwrap()).to_string();
        let hash160 = hash160::Hash::hash(&hex::decode(preimage.clone()).unwrap()).to_string();
        let preimage_bytes: Vec<u8> = hex::decode(preimage.clone()).unwrap();

        let sha256_bytes =
            sha256::Hash::hash(&hex::decode(preimage.clone()).unwrap()).to_byte_array();
        let hash160_bytes =
            hash160::Hash::hash(&hex::decode(preimage.clone()).unwrap()).to_byte_array();

        PreimageStates {
            preimage: Some(preimage),
            sha256,
            hash160,
            preimage_bytes: Some(preimage_bytes),
            sha256_bytes,
            hash160_bytes,
        }
    }

    pub fn from_str(preimage: &str) -> PreimageStates {
        let sha256 = sha256::Hash::hash(&hex::decode(preimage).unwrap()).to_string();
        let hash160 = hash160::Hash::hash(&hex::decode(preimage).unwrap()).to_string();
        let preimage_bytes: Vec<u8> = hex::decode(preimage).unwrap();

        let sha256_bytes = sha256::Hash::hash(&hex::decode(preimage).unwrap()).to_byte_array();
        let hash160_bytes = hash160::Hash::hash(&hex::decode(preimage).unwrap()).to_byte_array();

        PreimageStates {
            preimage: Some(preimage.to_string()),
            preimage_bytes: Some(preimage_bytes),
            sha256,
            hash160,
            sha256_bytes,
            hash160_bytes,
        }
    }

    pub fn from_sha256_str(preimage_sha256: &str) -> PreimageStates {
        let sha256 = sha256::Hash::from_str(preimage_sha256).unwrap();
        let hash160 = ripemd160::Hash::hash(sha256.as_byte_array());

        PreimageStates {
            preimage: None,
            sha256: sha256.to_string(),
            hash160: hash160.to_string(),
            preimage_bytes: None,
            sha256_bytes: sha256.to_byte_array(),
            hash160_bytes: hash160.to_byte_array(),
        }
    }
    pub fn from_invoice_str(invoice_str: &str) -> Result<PreimageStates, S5Error> {
        let invoice = match Bolt11Invoice::from_str(&invoice_str) {
            Ok(invoice) => invoice,
            Err(e) => {
                println!("{:?}", e);
                return Err(S5Error::new(
                    ErrorKind::Input,
                    "Could not parse invoice string.",
                ));
            }
        };

        Ok(PreimageStates::from_sha256_str(
            &invoice.payment_hash().to_string(),
        ))
    }
}