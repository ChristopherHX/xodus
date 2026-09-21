use crate::auth::do_sisu;
use crate::models::live::ExchangeUserTokenOutcome;
use crate::models::secrets::{LegacyToken, Token};
use crate::models::soap;
use crate::models::xbox::{
    Account, Atoms, Blob, Container, ContainerResponse, ContextDescription, Data, Title,
    XbConnectedStorageSpace, XstsResponse,
};
use crate::tokens::TokenManager;

pub mod auth;
pub mod title;
pub use auth::{authenticate_xbox_user, get_xsts_auth_header, request_xsts_token};
use base64::Engine;
use reqwest::Client;

pub async fn run(
    client: &reqwest::Client,
    dev_token: LegacyToken,
    legacy: LegacyToken,
    relying_party: &str,
) -> XstsResponse {
    let user_token = crate::api::live::exchange_user_token(
        client,
        legacy,
        "USERNAME".to_string(),
        dev_token,
        None,
        Some("Silent".to_string()),
        "{d6d5a677-0872-4ab0-9442-bb792fce85c5}".to_string(),
        &[(
            "user.auth.xboxlive.com".to_owned(),
            Some(soap::PolicyReference::mbi_ssl()),
        )],
    )
    .await
    .expect("Failed to get ms user token");

    let user_token: Token = match user_token {
        ExchangeUserTokenOutcome::Fault(_) => {
            eprintln!("Failed to get exchange MS token");
            panic!("TODO");
        }
        ExchangeUserTokenOutcome::Issued(
            soap::BodyContent::RequestSecurityTokenResponseCollection(mut collection),
        ) => {
            let token = collection.security_tokens.remove(0);
            token.into()
        }
        ExchangeUserTokenOutcome::Issued(soap::BodyContent::RequestSecurityTokenResponse(
            token,
        )) => (*token).into(),
        _ => unreachable!("Only responses are handled"),
    };
    let Token::Compact(user_token) = user_token else {
        eprintln!("Unsupported token");
        panic!("TODO");
    };
    let resp = authenticate_xbox_user(client, user_token)
        .await
        .expect("Failed to authenticate Xbox user");

    request_xsts_token(client, resp.token, relying_party)
        .await
        .expect("Failed to authenticate Xbox user")
}

pub async fn lock_container(
    client: &Client,
    token: &str,
    xuid: &str,
    scid: &str,
    pfn: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let r = client
        .put(
            format!("https://titlestorage.xboxlive.com/connectedstorage/users/xuid({xuid})/scids/{scid}/lock?friendlyName=linux"),
        )
        .header("x-xbl-contract-version", "2")
        .header("Authorization", token)
        // .header("Accept-Language", "en-US")// Required for no http 400
        .header("x-xbl-pfn", pfn)
        .header("x-xbl-lock-ver", "1")
        .header("x-xbl-lock-ext", "300")
        .send()
        .await?
        .error_for_status()?;
    let t = r.text().await?;

    Ok(t)
}

pub async fn delete_container(
    client: &Client,
    token: &str,
    xuid: &str,
    scid: &str,
    pfn: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let r = client
        .delete(
            format!("https://titlestorage.xboxlive.com/connectedstorage/users/xuid({xuid})/scids/{scid}/lock?friendlyName=linux"),
        )
        .header("x-xbl-contract-version", "2")
        .header("Authorization", token)
        // .header("Accept-Language", "en-US")// Required for no http 400
        .header("x-xbl-pfn", pfn)
        .header("x-xbl-lock-ver", "1")
        .header("x-xbl-lock-ext", "300")
        .send()
        .await?
        .error_for_status()?;

    let t = r.text().await?;

    Ok(t)
}

pub async fn fetch_containers(
    client: &Client,
    token: &str,
    xuid: &str,
    scid: &str,
) -> Result<ContainerResponse, Box<dyn std::error::Error>> {
    let r = client
        .get(format!(
            "https://titlestorage.xboxlive.com/connectedstorage/users/xuid({xuid})/scids/{scid}"
        ))
        .header("x-xbl-contract-version", "2")
        .header("Authorization", token)
        .header("Accept-Language", "en-US") // Required for no http 400
        .header("x-xbl-pfn", "Microsoft.BadgerWin10_8wekyb3d8bbwe")
        .send()
        .await?
        .error_for_status()?;

    let t = r.json::<ContainerResponse>().await?;

    Ok(t)
}

pub async fn fetch_container(
    client: &Client,
    token: &str,
    xuid: &str,
    scid: &str,
    container_name: &str,
    pfn: &str,
) -> Result<Atoms, Box<dyn std::error::Error>> {
    let r = client
        .get(
            format!("https://titlestorage.xboxlive.com/connectedstorage/users/xuid({xuid})/scids/{scid}/savedgames/{container_name}"),
        )
        .header("x-xbl-contract-version", "2")
        .header("Authorization", token)
        .header("Accept-Language", "en-US")// Required for no http 400
        .header("x-xbl-pfn", pfn)
        .send()
        .await?
        .error_for_status()?;

    let t = r.json::<Atoms>().await?;

    Ok(t)
}

pub async fn fetch_atom(
    client: &Client,
    token: &str,
    xuid: &str,
    scid: &str,
    atom: &str,
    pfn: &str,
) -> Result<bytes::Bytes, Box<dyn std::error::Error>> {
    let r = client
        .get(
            format!("https://titlestorage.xboxlive.com/connectedstorage/users/xuid({xuid})/scids/{scid}/{atom},binary"),
        )
        .header("x-xbl-contract-version", "2")
        .header("Authorization", token)
        .header("Accept-Language", "en-US")// Required for no http 400
        .header("x-xbl-pfn", pfn)
        .send()
        .await?
        .error_for_status()?;

    let t = r.bytes().await?;

    Ok(t)
}

pub async fn export_connected_storage_xml(
    client: &Client,
    tokens: &TokenManager,
    client_id: &str,
    title_id: i64,
    pfn: &str,
    scid: Option<&str>,
) -> XbConnectedStorageSpace {
    let scid = scid.map_or_else(
        || uuid::Uuid::from_u64_pair(0, title_id as u64).to_string(),
        |v| v.to_owned(),
    );

    let (mut a, resp, dt) = do_sisu(&client, &tokens, client_id, title_id)
        .await
        .expect("ok");

    let xuid = &resp
        .authorization_token
        .display_claims
        .as_ref()
        .unwrap()
        .xui[0]["xid"];

    let ut = a
        .get_xsts_token(
            Some(&dt),
            None,
            Some(&resp.user_token),
            "http://xboxlive.com",
        )
        .await
        .unwrap();

    let mut out_containers = Vec::<Container>::new();

    let containers = fetch_containers(&client, &ut.authorization_header_value(), xuid, &scid)
        .await
        .unwrap();
    for e in &containers.blobs {
        let mut blobs = Vec::<Blob>::new();
        let cn = e.file_name.strip_suffix(",savedgame").unwrap();
        let ci = fetch_container(
            &client,
            &ut.authorization_header_value(),
            xuid,
            &scid,
            cn,
            pfn,
        )
        .await
        .unwrap();
        for a in &ci.atoms {
            let ac = fetch_atom(
                &client,
                &ut.authorization_header_value(),
                xuid,
                &scid,
                &a.atom,
                pfn,
            )
            .await
            .unwrap();
            blobs.push(Blob {
                name: a.name.to_owned(),
                data: base64::engine::general_purpose::STANDARD.encode(ac),
            });
        }
        out_containers.push(Container {
            name: cn.to_owned(),
            display_name: e.display_name.to_string(),
            blobs: blobs,
        });
    }

    XbConnectedStorageSpace {
        context_description: ContextDescription {
            account: Account {
                msa: "me".to_owned(),
            },
            title: Title { scid },
        },
        data: Data {
            containers: out_containers,
        },
    }
}
