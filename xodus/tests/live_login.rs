use std::fs;
use std::path::PathBuf;

use base64::prelude::*;
use reqwest::Client;
use rsa::{RsaPrivateKey, traits::PublicKeyParts};
use serde::Serialize;
use xodus::api::live;
use xodus::api::live::ExchangeDeviceTokenOptions;
use xodus::api::live::ExchangeUserTokenOptions;
use xodus::licensing::utils::generate_string;
use xodus::models::devicecredential::{DeviceAddRequest, KeyValue, RsaKeyValue, TpmInfo};
use xodus::models::live::ExchangeUserTokenOutcome;
use xodus::models::secrets::Token as SecretToken;
use xodus::models::soap::BodyContent;
use xodus::tokens::TokenManager;
use xal::cvlib::CorrelationVector;
use xal::extensions::{
    CorrelationVectorReqwestBuilder, JsonExDeserializeMiddleware, SigningReqwestBuilder,
};
use xal::request::{XADProperties, XTokenRequest};
use xal::response::{DeviceToken, TitleEndpointsResponse};
use xal::{
    Constants, DeviceType, Error, RequestSigner, SignaturePolicyCache,
};

const HAR_USER_AGENT: &str = "XAL GAMERUNTIME 2025.07.20250718.000";
const HAR_DEVICE_VERSION: &str = "10.0.22621";
const TOKENBROKER_CLIENT_ID: &str = "{28C08266-F973-4AE6-FFE4-409B249F138F}";
const TOKENBROKER_SCOPE: &str = "scope=service::user.auth.xboxlive.com::MBI_SSL&api-version=2.0";
const FIXTURE_TOKENBROKER_HOSTING_APP: &str = "{fc177c6f-a3d6-4bb0-b1fa-23d0cd9b005d}";
const FIXTURE_TOKENBROKER_PACKAGE_SID: &str =
    "S-1-15-2-283421221-3183566570-1718213290-751554359-3541592344-2312209569-3374928651";
const FIXTURE_WINDOWS_CLIENT_STRING: &str = "Jkdh4zhtxPOMrTEt1VdNxpIv69KZf0LYmRXRrA6rcvY=";
const FIXTURE_REQUEST_PARAMS: &str = "AQAAAAIAAABsYwQAAAAxMDMz";
const HAR_SILENT_HOSTING_APP: &str = "{83928489-55ae-4c23-94ec-03a106b80ba2}";
const HAR_SILENT_PACKAGE_SID: &str =
    "S-1-15-2-1723189366-2159580849-2248400763-1481059666-1951766778-2756563051-3565589001";
const HAR_SILENT_WINDOWS_CLIENT_STRING: &str = "UEKuq3xDqBtji6WR9+2KfISn8uKvYUmwePMrRBfVtTs=";
const HAR_SILENT_REQUEST_PARAMS: &str = "AQAAAAIAAABsYwQAAAAxMDMx";
const HAR_OPENID_SCOPE: &str = "scope=openid&api-version=2.0&oauth2_response=1&uaid=f9e55347-ab7e-4372-8ca1-70402b88e9fb&claims=%7B%22id_token%22%3A%7B%22xms_agvs%22%3A%7B%22essential%22%3Afalse%2C%22additionalProperties%22%3A%5B%22interrupt%22%5D%7D%7D%7D";

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct XadRpsProperties<'a> {
    auth_method: &'a str,
    rps_ticket: &'a str,
    site_name: &'a str,
    version: &'a str,
    device_model: &'a str,
    proof_key: xal::ProofKey,
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/login-live-pairs")
}

fn load_device_add_template() -> DeviceAddRequest {
    let xml = fs::read_to_string(fixture_root().join("0002/request-body.xml"))
        .expect("missing device-add fixture request");
    quick_xml::de::from_str(&xml).expect("failed to parse device-add fixture request")
}

fn provision_request_from_fixture_template() -> (DeviceAddRequest, String, String, RsaPrivateKey) {
    let mut request = load_device_add_template();
    let username = format!("02{}", generate_string(14));
    let password = generate_string(20);

    request.authentication.membername = username.clone();
    request.authentication.password = password.clone();

    let private_key =
        RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).expect("failed to generate RSA key");
    // let public_key = private_key.to_public_key();

    let device_info = request
        .device_info
        .as_mut()
        .expect("fixture request is missing DeviceInfo");
    // device_info.tpm_info = Some(TpmInfo {
    //     key_value: KeyValue {
    //         rsa_key_value: RsaKeyValue {
    //             modulus: BASE64_STANDARD.encode(public_key.n().to_bytes_be()),
    //             exponent: BASE64_STANDARD.encode(public_key.e().to_bytes_be()),
    //         },
    //     },
    // });
    device_info.tpm_info = None;

    (request, username, password, private_key)
}

fn expect_device_auth_response(
    auth: xodus::models::soap::Envelope,
) -> xodus::models::soap::RequestSecurityTokenResponse {
    match auth.body.body {
        BodyContent::RequestSecurityTokenResponse(response) => response,
        body => panic!("unexpected authenticate_device response body: {body:?}"),
    }
}

fn expect_compact_token(token: xodus::models::soap::RequestSecurityTokenResponse) -> String {
    match SecretToken::from(token) {
        SecretToken::Compact(token) => token,
        other => panic!("expected compact token, got {other:?}"),
    }
}

fn extract_compact_token_from_body(body: BodyContent, expected_scope: &str) -> String {
    match body {
        BodyContent::RequestSecurityTokenResponse(response) => {
            assert_eq!(
                response.applies_to.endpoint_reference.address,
                expected_scope,
                "unexpected compact token scope",
            );
            expect_compact_token(response)
        }
        BodyContent::RequestSecurityTokenResponseCollection(collection) => collection
            .security_tokens
            .into_iter()
            .find_map(|response| {
                (response.applies_to.endpoint_reference.address == expected_scope)
                    .then(|| expect_compact_token(response))
            })
            .unwrap_or_else(|| panic!("no compact token issued for scope {expected_scope}")),
        other => panic!("unexpected exchange_user_token body: {other:?}"),
    }
}

fn build_xal_client() -> Result<Client, Error> {
    Client::builder()
        .user_agent(HAR_USER_AGENT)
        .build()
        .map_err(Into::into)
}

async fn fetch_title_endpoints(client: &Client) -> Result<TitleEndpointsResponse, Error> {
    client
        .get(Constants::XBOX_TITLE_ENDPOINTS_URL)
        .header("x-xbl-contract-version", "1")
        .query(&[("type", "1")])
        .send()
        .await?
        .json_ex::<TitleEndpointsResponse>()
        .await
}

async fn get_xbox_device_token_like_replay(
    client: &Client,
    signer: &mut RequestSigner,
    cv: &mut CorrelationVector,
) -> Result<DeviceToken, Error> {
    let device_id = format!(
        "{{{}}}",
        uuid::Uuid::new_v4().hyphenated().to_string().to_uppercase()
    );
    let body = XTokenRequest::<XADProperties> {
        relying_party: Constants::RELYING_PARTY_AUTH_XBOXLIVE,
        token_type: "JWT",
        properties: XADProperties {
            auth_method: "ProofOfPossession",
            id: &device_id,
            device_type: &DeviceType::WIN32.to_string(),
            version: HAR_DEVICE_VERSION,
            proof_key: signer.get_proof_key(),
        },
    };

    client
        .post(Constants::XBOX_DEVICE_AUTH_URL)
        .header("x-xbl-contract-version", "1")
        .add_cv(cv)?
        .json(&body)
        .sign(signer, None)
        .await?
        .send()
        .await?
        .json_ex::<DeviceToken>()
        .await
}

async fn get_xbox_device_token_rps_like_new_xal(
    client: &Client,
    signer: &mut RequestSigner,
    cv: &mut CorrelationVector,
    rps_ticket: &str,
    site_name: &str,
    contract_version: &str,
    device_model: &str,
) -> Result<DeviceToken, Error> {
    let body = XTokenRequest::<XadRpsProperties> {
        relying_party: Constants::RELYING_PARTY_AUTH_XBOXLIVE,
        token_type: "JWT",
        properties: XadRpsProperties {
            auth_method: "RPS",
            rps_ticket,
            site_name,
            version: HAR_DEVICE_VERSION,
            device_model,
            proof_key: signer.get_proof_key(),
        },
    };

    let response = client
        .post(Constants::XBOX_DEVICE_AUTH_URL)
        .header("x-xbl-contract-version", contract_version)
        .add_cv(cv)?
        .json(&body)
        .sign(signer, None)
        .await?
        .send()
        .await?;

    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(Error::GeneralError(format!(
            "device.auth failed status={status} body={text}"
        )));
    }

    serde_json::from_str::<DeviceToken>(&text)
        .map_err(|err| Error::GeneralError(format!("device.auth JSON parse failed: {err}; body={text}")))
}

#[tokio::test]
#[ignore = "makes real requests to login.live.com"]
async fn live_device_flow_from_fixture_template() {
    let client = reqwest::Client::builder()
        .build()
        .expect("failed to build reqwest client");

    let (request, username, password, _private_key) = provision_request_from_fixture_template();

    let provision = live::login_device_credential(&client, request)
        .await
        .expect("device provisioning request failed");
    assert!(provision.success, "device provisioning was not successful");
    assert!(
        !provision.puid.is_empty(),
        "device provisioning response did not include a PUID"
    );

    let auth = live::authenticate_device(&client, username, password)
        .await
        .expect("device authentication request failed");
    let response = expect_device_auth_response(auth);

    assert!(
        !response.token_type.is_empty(),
        "STS response token type should not be empty"
    );
}

#[tokio::test]
#[ignore = "makes real requests to login.live.com"]
async fn live_device_token_exchange_using_live_device_credentials() {
    let client = reqwest::Client::builder()
        .build()
        .expect("failed to build reqwest client");

    let (request, username, password, _private_key) = provision_request_from_fixture_template();

    let _provision = live::login_device_credential(&client, request)
        .await
        .expect("device provisioning request failed");

    let auth = live::authenticate_device(&client, username, password)
        .await
        .expect("device authentication request failed");
    let response = expect_device_auth_response(auth);

    let legacy_device_token = match SecretToken::from(response) {
        SecretToken::Legacy(token) => token,
        other => panic!("expected legacy device token, got {other:?}"),
    };
    let device_binary_secret = legacy_device_token
        .binary_secret
        .clone()
        .expect("device auth response did not include a binary secret");
    let compact_device_token = expect_compact_token(
        live::exchange_device_token(
            &client,
            legacy_device_token.token,
            device_binary_secret,
            "{d6d5a677-0872-4ab0-9442-bb792fce85c5}".to_string(),
            "user.auth.xboxlive.com".to_string(),
            Some(xodus::models::soap::PolicyReference::mbi_ssl()),
        )
        .await
        .expect("failed to exchange Microsoft device token"),
    );

    assert!(
        !compact_device_token.is_empty(),
        "device token exchange did not return a compact token"
    );
    assert!(
        compact_device_token.starts_with("t="),
        "compact device token does not look like an MBI ticket"
    );
}

#[tokio::test]
#[ignore = "makes real requests to xboxlive.com"]
async fn live_xbox_device_auth_like_replay_failed_sisu_once() {
    let client = build_xal_client().expect("failed to build XAL client");
    let endpoints = fetch_title_endpoints(&client)
        .await
        .expect("failed to fetch title endpoints");

    let mut signer = RequestSigner::new();
    signer.signature_policy_cache = SignaturePolicyCache::new(endpoints);
    let mut cv = CorrelationVector::new();

    let device_token = get_xbox_device_token_like_replay(&client, &mut signer, &mut cv)
        .await
        .expect("failed to get Xbox device token");

    assert!(
        !device_token.token.is_empty(),
        "Xbox device auth did not return a token"
    );
    assert!(
        device_token.display_claims.is_some(),
        "Xbox device auth did not return device display claims"
    );
}

#[tokio::test]
#[ignore = "makes real requests to login.live.com and xboxlive.com"]
async fn live_xbox_device_auth_via_rps_ticket_from_live_device_flow() {
    let live_client = reqwest::Client::builder()
        .build()
        .expect("failed to build login.live.com client");

    let (request, username, password, _private_key) = provision_request_from_fixture_template();

    let _provision = live::login_device_credential(&live_client, request)
        .await
        .expect("device provisioning request failed");

    let auth = live::authenticate_device(&live_client, username, password)
        .await
        .expect("device authentication request failed");
    let response = expect_device_auth_response(auth);

    let legacy_device_token = match SecretToken::from(response) {
        SecretToken::Legacy(token) => token,
        other => panic!("expected legacy device token, got {other:?}"),
    };
    let device_binary_secret = legacy_device_token
        .binary_secret
        .clone()
        .expect("device auth response did not include a binary secret");
    let exchange_attempts = [
        (
            "current-defaults",
            &live_client,
            ExchangeDeviceTokenOptions {
                sso_flags: Some("SsoRestr".to_string()),
                ..ExchangeDeviceTokenOptions::default()
            },
        )
        ,
        (
            "fixture-tokenbroker",
            &live_client,
            ExchangeDeviceTokenOptions {
                sso_flags: None,
                hosting_app: Some(FIXTURE_TOKENBROKER_HOSTING_APP.to_string()),
                inline_ux: Some("TokenBroker".to_string()),
                package_sid: Some(FIXTURE_TOKENBROKER_PACKAGE_SID.to_string()),
                request_params: Some(FIXTURE_REQUEST_PARAMS.to_string()),
                windows_client_string: Some(FIXTURE_WINDOWS_CLIENT_STRING.to_string()),
                binary_version: Some("45".to_string()),
            },
        ),
    ];

    let client = build_xal_client().expect("failed to build XAL client");
    let endpoints = fetch_title_endpoints(&client)
        .await
        .expect("failed to fetch title endpoints");

    let mut signer = RequestSigner::new();
    signer.signature_policy_cache = SignaturePolicyCache::new(endpoints);
    let mut cv = CorrelationVector::new();

    let mut failures = Vec::new();

    for (exchange_name, exchange_client, exchange_options) in exchange_attempts {
        let compact_device_token = expect_compact_token(
            live::exchange_device_token_with_options(
                exchange_client,
                legacy_device_token.token.clone(),
                device_binary_secret.clone(),
                TOKENBROKER_CLIENT_ID.to_string(),
                TOKENBROKER_SCOPE.to_string(),
                Some(xodus::models::soap::PolicyReference::token_broker()),
                exchange_options,
            )
            .await
            .expect("failed to exchange Microsoft device token"),
        );

        println!(
            "RPS ticket metadata exchange={exchange_name}: len={} starts_with_t={} contains_p={}",
            compact_device_token.len(),
            compact_device_token.starts_with("t="),
            compact_device_token.contains("&p="),
        );

        let attempts = [
            (
                "tokenbroker-baseline",
                "user.auth.xboxlive.com",
                "1",
                "XPS 13 9370",
            ),
            (
                "tokenbroker-contract-v2",
                "user.auth.xboxlive.com",
                "2",
                "XPS 13 9370",
            ),
            (
                "device-site-contract-v1",
                "device.auth.xboxlive.com",
                "1",
                "XPS 13 9370",
            ),
            (
                "device-site-contract-v2",
                "device.auth.xboxlive.com",
                "2",
                "XPS 13 9370",
            ),
            (
                "empty-device-model",
                "user.auth.xboxlive.com",
                "1",
                "",
            ),
        ];

        for (name, site_name, contract_version, device_model) in attempts {
            println!(
                "Trying exchange={exchange_name} attempt={name} site_name={site_name} contract_version={contract_version} device_model={device_model:?}"
            );
            match get_xbox_device_token_rps_like_new_xal(
                &client,
                &mut signer,
                &mut cv,
                &compact_device_token,
                site_name,
                contract_version,
                device_model,
            )
            .await
            {
                Ok(device_token) => {
                    println!(
                        "Success exchange={exchange_name} attempt={name} token_len={} has_claims={}",
                        device_token.token.len(),
                        device_token.display_claims.is_some()
                    );
                    assert!(
                        !device_token.token.is_empty(),
                        "RPS-based Xbox device auth returned an empty token"
                    );
                    assert!(
                        device_token.display_claims.is_some(),
                        "RPS-based Xbox device auth did not return display claims"
                    );
                    return;
                }
                Err(err) => {
                    println!("Failed exchange={exchange_name} attempt={name}: {err:?}");
                    failures.push(format!(
                        "exchange={exchange_name} {name} site_name={site_name} contract_version={contract_version} device_model={device_model:?}: {err:?}"
                    ));
                }
            }
        }
    }

    panic!(
        "all RPS device-auth attempts failed:\n{}",
        failures.join("\n")
    );
}

#[tokio::test]
#[ignore = "makes real requests using locally stored user/device STS tokens"]
async fn live_xbox_device_auth_via_rps_ticket_from_stored_user_and_device_tokens() {
    xodus::secrets::init_secrets().expect("failed to initialize local secret store");

    let tokens = TokenManager::with_keychain_and_memory();
    let device_sts = match tokens.get_device_sts_token().expect("missing stored device STS token") {
        SecretToken::Legacy(token) => token,
        other => panic!("expected legacy stored device STS token, got {other:?}"),
    };
    let user_sts = match tokens.get_user_sts_token().expect("missing stored user STS token") {
        SecretToken::Legacy(token) => token,
        other => panic!("expected legacy stored user STS token, got {other:?}"),
    };
    let user = tokens.get_user().expect("missing stored user info");

    let live_client = reqwest::Client::builder()
        .build()
        .expect("failed to build login.live.com client");

    let client = build_xal_client().expect("failed to build XAL client");
    let endpoints = fetch_title_endpoints(&client)
        .await
        .expect("failed to fetch title endpoints");

    let mut signer = RequestSigner::new();
    signer.signature_policy_cache = SignaturePolicyCache::new(endpoints);
    let mut cv = CorrelationVector::new();
    let exchange_attempts = [
        (
            "user-auth-defaults",
            TOKENBROKER_SCOPE,
            TOKENBROKER_SCOPE,
            ExchangeUserTokenOptions::default(),
        ),
        (
            "user-auth-silent-har-shape",
            TOKENBROKER_SCOPE,
            TOKENBROKER_SCOPE,
            ExchangeUserTokenOptions {
                sso_flags: None,
                hosting_app: Some(HAR_SILENT_HOSTING_APP.to_string()),
                inline_ux: Some("Silent".to_string()),
                package_sid: Some(HAR_SILENT_PACKAGE_SID.to_string()),
                request_params: Some(HAR_SILENT_REQUEST_PARAMS.to_string()),
                windows_client_string: Some(HAR_SILENT_WINDOWS_CLIENT_STRING.to_string()),
                binary_version: Some("45".to_string()),
            },
        ),
        (
            "openid-silent-har-shape",
            HAR_OPENID_SCOPE,
            HAR_OPENID_SCOPE,
            ExchangeUserTokenOptions {
                sso_flags: None,
                hosting_app: Some(HAR_SILENT_HOSTING_APP.to_string()),
                inline_ux: Some("Silent".to_string()),
                package_sid: Some(HAR_SILENT_PACKAGE_SID.to_string()),
                request_params: Some(HAR_SILENT_REQUEST_PARAMS.to_string()),
                windows_client_string: Some(HAR_SILENT_WINDOWS_CLIENT_STRING.to_string()),
                binary_version: Some("45".to_string()),
            },
        ),
    ];

    let mut failures = Vec::new();

    for (exchange_name, requested_scope, expected_scope, exchange_options) in exchange_attempts {
        let exchanged = live::exchange_user_token_with_options(
            &live_client,
            user_sts.token.clone(),
            user.username.clone(),
            device_sts.token.clone(),
            device_sts
                .binary_secret
                .clone()
                .expect("stored device STS token is missing its shared secret"),
            None,
            None,
            TOKENBROKER_CLIENT_ID.to_string(),
            &[(
                requested_scope.to_string(),
                Some(xodus::models::soap::PolicyReference::token_broker()),
            )],
            exchange_options,
        )
        .await
        .expect("exchange_user_token request failed");

        let compact_rps_ticket = match exchanged {
            ExchangeUserTokenOutcome::Issued(body) => {
                extract_compact_token_from_body(body, expected_scope)
            }
            ExchangeUserTokenOutcome::Fault(pp) => {
                println!("Stored-token exchange fault exchange={exchange_name}: {pp:?}");
                failures.push(format!("exchange={exchange_name} fault={pp:?}"));
                continue;
            }
        };

        println!(
            "Stored-token RPS ticket metadata exchange={exchange_name}: len={} starts_with_t={} contains_p={}",
            compact_rps_ticket.len(),
            compact_rps_ticket.starts_with("t="),
            compact_rps_ticket.contains("&p="),
        );

        match get_xbox_device_token_rps_like_new_xal(
            &client,
            &mut signer,
            &mut cv,
            &compact_rps_ticket,
            "user.auth.xboxlive.com",
            "1",
            "XPS 13 9370",
        )
        .await
        {
            Ok(device_token) => {
                assert!(
                    !device_token.token.is_empty(),
                    "stored-token device.auth returned an empty token"
                );
                assert!(
                    device_token.display_claims.is_some(),
                    "stored-token device.auth did not return display claims"
                );
                return;
            }
            Err(err) => {
                println!("Stored-token device.auth failure exchange={exchange_name}: {err:?}");
                failures.push(format!("exchange={exchange_name} device.auth err={err:?}"));
            }
        }
    }

    panic!(
        "all stored-token exchange attempts failed:\n{}",
        failures.join("\n")
    );
}
