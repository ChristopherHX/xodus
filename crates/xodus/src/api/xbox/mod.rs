use crate::auth::do_sisu;
use crate::models::live::ExchangeUserTokenOutcome;
use crate::models::secrets::{LegacyToken, Token};
use crate::models::soap;
use crate::models::xbox::XstsResponse;
use crate::tokens::TokenManager;

pub mod auth;
pub mod title;
pub use auth::{authenticate_xbox_user, get_xsts_auth_header, request_xsts_token};
use base64::Engine;
use reqwest::Client;
use serde::Serialize;
use serde::Deserialize;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

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

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PagingInfo {
    pub continuation_token: Option<String>,
    pub total_items: i64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ContainerBlob {
    pub client_file_time: String,
    pub display_name: String,
    pub etag: String,
    pub file_name: String,
    pub size: i64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ContainerResponse {
    pub blobs: Vec<ContainerBlob>,
    pub paging_info: PagingInfo,
}


#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Atom {
    pub atom: String,
    pub name: String,
    pub size: i64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Atoms {
    pub atoms: Vec<Atom>,
}
// {
//     blobs: [
//         // For each container
//         {
//             "clientFileTime": <lastModifiedTime>
//             "displayName": <containerDisplayName>
//             "etag": <not exposed to the API but saved in containers.index>
//             "fileName": <containerName>,savedGame
//             "size": <totalSize>
//         }
//     ],
//     pagingInfo: {
//         "continuationToken": null,
//         "totalItems" <number of containers>
//     }
// }

// { "ownerChangedId":<GUID>, "quotaBytes":<Quota> }

pub async fn lock_container(
    client: &Client,
    token: &str,
    xuid: &str,
    scid: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let r = client
        .put(
            format!("https://titlestorage.xboxlive.com/connectedstorage/users/xuid({xuid})/scids/{scid}/lock?friendlyName=linux"),
        )
        .header("x-xbl-contract-version", "2")
        .header("Authorization", token)
        // .header("Accept-Language", "en-US")// Required for no http 400
        .header("x-xbl-pfn", "Microsoft.BadgerWin10_8wekyb3d8bbwe")
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
) -> Result<String, Box<dyn std::error::Error>> {
    let r = client
        .delete(
            format!("https://titlestorage.xboxlive.com/connectedstorage/users/xuid({xuid})/scids/{scid}/lock?friendlyName=linux"),
        )
        .header("x-xbl-contract-version", "2")
        .header("Authorization", token)
        // .header("Accept-Language", "en-US")// Required for no http 400
        .header("x-xbl-pfn", "Microsoft.BadgerWin10_8wekyb3d8bbwe")
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
        .get(
            format!("https://titlestorage.xboxlive.com/connectedstorage/users/xuid({xuid})/scids/{scid}"),
        )
        .header("x-xbl-contract-version", "2")
        .header("Authorization", token)
        .header("Accept-Language", "en-US")// Required for no http 400
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
) -> Result<Atoms, Box<dyn std::error::Error>> {
    let r = client
        .get(
            format!("https://titlestorage.xboxlive.com/connectedstorage/users/xuid({xuid})/scids/{scid}/savedgames/{container_name}"),
        )
        .header("x-xbl-contract-version", "2")
        .header("Authorization", token)
        .header("Accept-Language", "en-US")// Required for no http 400
        .header("x-xbl-pfn", "Microsoft.BadgerWin10_8wekyb3d8bbwe")
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
) -> Result<bytes::Bytes, Box<dyn std::error::Error>> {
    let r = client
        .get(
            format!("https://titlestorage.xboxlive.com/connectedstorage/users/xuid({xuid})/scids/{scid}/{atom},binary"),
        )
        .header("x-xbl-contract-version", "2")
        .header("Authorization", token)
        .header("Accept-Language", "en-US")// Required for no http 400
        .header("x-xbl-pfn", "Microsoft.BadgerWin10_8wekyb3d8bbwe")
        .send()
        .await?
        .error_for_status()?;

    let t = r.bytes().await?;

    Ok(t)
}

#[derive(Serialize, Deserialize, Debug)]
struct Account {
    #[serde(rename = "@msa")]
    msa: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Title {
    #[serde(rename = "@scid")]
    scid: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ContextDescription {
    account: Account,
    title: Title,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Blob {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "$text")]
    data: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Container {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@displayName")]
    display_name: String,
    blobs: Vec<Blob>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Data {
    containers: Vec<Container>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct XbConnectedStorageSpace {
    context_description: ContextDescription,
    data: Data,
}

#[ignore]
#[tokio::test]
async fn test_serialize() {
    let mut writer = String::new();
    let mut ser = quick_xml::se::Serializer::new(&mut writer);
    ser.text_format(quick_xml::se::TextFormat::CData);
    let data = XbConnectedStorageSpace{
        context_description: ContextDescription { account: Account { msa: "me".to_owned() }, title: Title { scid: "000".to_owned() } },
        data: Data { containers: vec![Container{blobs: vec![Blob{ name: "myblob".to_owned(), data: "binary Datac <xml></xml> --> ]]>".to_owned() }], display_name: "me".to_owned(), name: "es".to_owned()}] },
    }.serialize(ser).unwrap();
    // quick_xml::se::Serializer::text_format(&mut self, format)
    println!("xs {writer}");
}

#[ignore]
#[tokio::test]
async fn test_title_access() {
    // let filter = tracing_subscriber::EnvFilter::from_env("XODUS_LOG");
    // let registry =
    //     tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().with_filter(filter));

    // {
    //     registry.init();
    // }
    let client = reqwest::Client::new();
    crate::secrets::init_secrets().expect("Unable to initialize credentials");
    let tokens = TokenManager::with_keychain_and_memory();

    let (mut a, resp, dt) = do_sisu(&client, &tokens, "000000004C5D37D8", 0x61B215AE)
        .await
        .expect("ok");

    println!("title {}", resp.title_token.token);
    println!("user {}", resp.user_token.token);
    println!("webpage {}", resp.web_page);

    // println!("at");
    // for (k, v) in &resp.authorization_token.display_claims.as_ref().unwrap().xui[0] {
    //     println!("{k}: {v}");
    // }

    // println!("ut");
    // for (k, v) in &resp.user_token.display_claims.as_ref().unwrap().xui[0] {
    //     println!("{k}: {v}");
    // }
    let xuid = &resp.authorization_token.display_claims.as_ref().unwrap().xui[0]["xid"];

    let ut = a.get_xsts_token(Some(&dt), None, Some(&resp.user_token), "http://xboxlive.com").await.unwrap();

    let mut out_containers = Vec::<Container>::new();
    
    println!("R {}", lock_container(&client, &ut.authorization_header_value(), xuid, "00000000-0000-0000-0000-000061B215AE").await.unwrap());
    let containers = fetch_containers(&client, &ut.authorization_header_value(), xuid, "00000000-0000-0000-0000-000061B215AE").await.unwrap();
    println!("{:?}", &containers);
    for e in &containers.blobs {
        let mut blobs = Vec::<Blob>::new();
        let cn = e.file_name.strip_suffix(",savedgame").unwrap();
        // let out_container = out_containers.push_mut(Container { name: (), display_name: (), blobs: () });
        let ci = fetch_container(&client, &ut.authorization_header_value(), xuid, "00000000-0000-0000-0000-000061B215AE", cn).await.unwrap();
        println!("{:?}", ci);
        for a in &ci.atoms {
            let ac = fetch_atom(&client, &ut.authorization_header_value(), xuid, "00000000-0000-0000-0000-000061B215AE", &a.atom).await.unwrap();
            // if e.display_name.ends_with(".txt") {
            //     println!("{}", e.display_name);
            //     println!("{}", ac);
            // }
            blobs.push(Blob { name: a.name.to_owned(), data: base64::engine::general_purpose::STANDARD.encode(ac) });
        }
        out_containers.push(Container { name: cn.to_owned(), display_name: e.display_name.to_string(), blobs: blobs });
    }
    println!("R {}", delete_container(&client, &ut.authorization_header_value(), xuid, "00000000-0000-0000-0000-000061B215AE").await.unwrap());

    let mut writer = String::new();
    let mut ser = quick_xml::se::Serializer::new(&mut writer);
    ser.text_format(quick_xml::se::TextFormat::CData);
    XbConnectedStorageSpace{
        context_description: ContextDescription { account: Account { msa: "me".to_owned() }, title: Title { scid: "00000000-0000-0000-0000-000061B215AE".to_owned() } },
        data: Data { containers: out_containers },
    }.serialize(ser).unwrap();
    // quick_xml::se::Serializer::text_format(&mut self, format)
    println!("{writer}");
}


#[ignore]
#[tokio::test]
async fn test_title_access2() {
    let filter = tracing_subscriber::EnvFilter::from_env("XODUS_LOG");
    let registry =
        tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().with_filter(filter));

    {
        registry.init();
    }
    let client = reqwest::Client::new();
    crate::secrets::init_secrets().expect("Unable to initialize credentials");
    let tokens = TokenManager::with_keychain_and_memory();

    let dev_token = tokens.get_device_sts_token().unwrap();
    let Token::Legacy(dev_token) = dev_token else {
        panic!();
    };
    let user_token = tokens.get_user_sts_token().unwrap();
    let Token::Legacy(legacy) = user_token else {
        panic!();
    };

    let xsts_token =
        crate::api::xbox::run(&client, dev_token, legacy, "http://xboxlive.com").await;
    
    let uhs = xsts_token.user_hash().unwrap();
    let tkn = format!("XBL3.0 x={};{}", uhs, xsts_token.token);

    println!("R {}", lock_container(&client, &tkn, "2535418015510202", "00000000-0000-0000-0000-000061B215AE").await.unwrap());

}
