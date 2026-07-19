use serde::{Deserialize, Serialize};

use crate::models::soap::{self, EncryptedKey, Timestamp};

#[derive(Debug, Serialize, Deserialize)]
pub struct Device {
    pub puid: String,
    pub hwid: String,
    pub device_id: String,
    pub splicense: String,
    pub username: String,
    pub password: String,
    pub private_key: rsa::RsaPrivateKey,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LegacyToken {
    pub key_name: Option<String>,
    pub token: String,
    pub binary_secret: Option<String>,
    // pub encrypted_key: Option<EncryptedKey>,
    pub lifetime: Timestamp,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum Token {
    Legacy(LegacyToken),
    Compact(String),
}

impl From<soap::RequestSecurityTokenResponse> for Token {
    fn from(value: soap::RequestSecurityTokenResponse) -> Self {
        match value.token_type.as_str() {
            "urn:passport:legacy" => {
                let encrypted_data = value.requested_security_token.encrypted_data.unwrap();
                let key_name = encrypted_data.key_info.key_name.clone();
                let token = quick_xml::se::to_string(&encrypted_data).unwrap();
                let mut binary_secret = None;
                // let mut encrypted_key = None;
                // if let Some((bs, es)) = value.requested_proof_token.map(|t| (t.binary_secret, t.encrypted_key)) {
                //     binary_secret = Some(bs);
                //     encrypted_key = Some(es);
                // }
                if let Some((bs)) = value.requested_proof_token.map(|t| (t.binary_secret)) {
                    binary_secret = Some(bs);
                }
                Self::Legacy(LegacyToken {
                    key_name,
                    token,
                    binary_secret: binary_secret,
                    // encrypted_key: encrypted_key,
                    lifetime: value.lifetime,
                })
            }
            "urn:passport:compact" => Self::Compact(
                value
                    .requested_security_token
                    .binary_security_token
                    .unwrap()
                    .value,
            ),
            "urn:passport:delegationcompact" => Self::Compact(format!(
                "d={}",
                value
                    .requested_security_token
                    .binary_security_token
                    .unwrap()
                    .value
            )),
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenStore {
    #[serde(flatten)]
    pub tokens: std::collections::HashMap<String, Token>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    pub puid: String,
    pub username: String,
}
