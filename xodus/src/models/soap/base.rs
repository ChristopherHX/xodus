use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MustUnderstandValue {
    #[serde(rename = "@s:mustUnderstand")]
    pub must_understand: Option<String>,
    #[serde(rename = "$value")]
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Fault {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timestamp {
    #[serde(rename = "@wsu:Id", alias = "@Id")]
    pub id: Option<String>,
    #[serde(rename = "wsu:Created", alias = "Created")]
    pub created: String,
    #[serde(rename = "wsu:Expires", alias = "Expires")]
    pub expires: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyIdentifier {
    #[serde(rename = "@ValueType")]
    pub value_type: String,
    #[serde(rename = "$value")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceUri {
    #[serde(rename = "@URI")]
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestedTokenReference {
    #[serde(rename = "wsse:KeyIdentifier", alias = "KeyIdentifier")]
    pub key_identifier: Option<KeyIdentifier>,
    #[serde(rename = "wsse:Reference", alias = "Reference")]
    pub reference: ReferenceUri,
}

impl Default for RequestedTokenReference {
    fn default() -> Self {
        Self {
            key_identifier: Some(KeyIdentifier {
                value_type: "http://docs.oasis-open.org/wss/2004/XX/oasis-2004XX-wss-saml-token-profile-1.0#SAMLAssertionID".to_string(),
                value: None,
            }),
            reference: ReferenceUri { uri: String::new() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_token_reference_deserializes_namespaced_children() {
        let xml = r#"<wsse:RequestedTokenReference xmlns:wsse="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd">
                        <wsse:KeyIdentifier ValueType="http://docs.oasis-open.org/wss/2004/XX/oasis-2004XX-wss-saml-token-profile-1.0#SAMLAssertionID"/>
                        <wsse:Reference URI=""/>
                    </wsse:RequestedTokenReference>"#;

        let reference: RequestedTokenReference =
            quick_xml::de::from_str(xml).expect("failed to deserialize requested token reference");

        assert_eq!(reference.reference.uri, "");
        assert_eq!(
            reference
                .key_identifier
                .expect("missing key identifier")
                .value_type,
            "http://docs.oasis-open.org/wss/2004/XX/oasis-2004XX-wss-saml-token-profile-1.0#SAMLAssertionID"
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliesTo {
    #[serde(rename = "wsa:EndpointReference", alias = "EndpointReference")]
    pub endpoint_reference: EndpointReference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointReference {
    #[serde(rename = "wsa:Address", alias = "Address")]
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyReference {
    #[serde(rename = "@URI")]
    pub uri: String,
    #[serde(rename = "$value", default)]
    pub val: String,
}

impl PolicyReference {
    pub fn token_broker() -> Self {
        Self {
            uri: "TOKEN_BROKER".to_string(),
            val: String::default(),
        }
    }

    pub fn mbi_ssl() -> Self {
        Self {
            uri: "mbi_ssl".to_string(),
            val: String::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PP {
    #[serde(
        rename = "@xmlns:psf",
        alias = "@xmlns",
        skip_serializing_if = "Option::is_none"
    )]
    pub xmlns_psf: Option<String>,

    #[serde(rename = "psf:serverVersion", alias = "serverVersion")]
    pub server_version: Option<String>,

    #[serde(rename = "psf:reqstatus", alias = "reqstatus")]
    pub req_status: Option<String>,

    #[serde(rename = "psf:inlineauthurl", alias = "inlineauthurl")]
    pub inline_auth_url: Option<String>,

    #[serde(rename = "psf:inlineendauthurl", alias = "inlineendauthurl")]
    pub inline_end_auth_url: Option<String>,

    #[serde(rename = "psf:authstateinternal", alias = "authstateinternal")]
    pub auth_state_internal: Option<String>,

    #[serde(rename = "psf:UserSessionKey", alias = "UserSessionKey")]
    pub user_session_key: Option<String>,
}
