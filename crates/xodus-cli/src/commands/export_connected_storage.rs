use serde_core::ser::Serialize;
use std::{fs::File, io::Write, process::ExitCode};
use xodus::{api::xbox::export_connected_storage_xml, tokens::TokenManager};

pub async fn run(
    client: &reqwest::Client,
    tokens: &TokenManager,
    msa_id: String,
    title_id: i64,
    pfn: &str,
    out: String,
    scid: Option<&str>,
) -> ExitCode {
    let mut file = File::create(out).unwrap();
    let data = export_connected_storage_xml(client, tokens, &msa_id, title_id, pfn, scid).await;
    let mut writer = String::new();
    let mut ser = quick_xml::se::Serializer::new(&mut writer);
    ser.text_format(quick_xml::se::TextFormat::CData);
    data.serialize(ser).unwrap();
    file.write_all(writer.as_bytes()).unwrap();
    ExitCode::SUCCESS
}
