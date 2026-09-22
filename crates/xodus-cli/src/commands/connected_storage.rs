use serde_core::ser::Serialize;
use std::io::Read;
use std::{fs::File, io::Write, process::ExitCode};
use xodus::api::xbox::upload_connected_storage_xml;
use xodus::models::xbox::XbConnectedStorageSpace;
use xodus::{api::xbox::download_connected_storage_xml, tokens::TokenManager};

pub async fn download(
    client: &reqwest::Client,
    tokens: &TokenManager,
    msa_id: String,
    title_id: i64,
    pfn: &str,
    out: String,
    scid: Option<&str>,
) -> ExitCode {
    let mut file = File::create(out).unwrap();
    let data = download_connected_storage_xml(client, tokens, &msa_id, title_id, pfn, scid)
        .await
        .unwrap();
    let mut writer = String::new();
    let mut ser = quick_xml::se::Serializer::new(&mut writer);
    ser.text_format(quick_xml::se::TextFormat::CData);
    data.serialize(ser).unwrap();
    file.write_all(writer.as_bytes()).unwrap();
    ExitCode::SUCCESS
}

pub async fn upload(
    client: &reqwest::Client,
    tokens: &TokenManager,
    msa_id: String,
    title_id: i64,
    pfn: &str,
    input: String,
    scid: Option<&str>,
    keep_existing: bool,
) -> ExitCode {
    let mut file = File::open(input).unwrap();
    let mut content = String::new();
    file.read_to_string(&mut content).unwrap();
    let storage: XbConnectedStorageSpace = quick_xml::de::from_str(&content).unwrap();
    let data = upload_connected_storage_xml(
        client,
        tokens,
        &msa_id,
        title_id,
        pfn,
        scid,
        storage,
        keep_existing,
    )
    .await
    .unwrap();
    let mut writer = String::new();
    let mut ser = quick_xml::se::Serializer::new(&mut writer);
    ser.text_format(quick_xml::se::TextFormat::CData);
    data.serialize(ser).unwrap();
    file.write_all(writer.as_bytes()).unwrap();
    ExitCode::SUCCESS
}
