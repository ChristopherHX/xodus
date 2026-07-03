use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use xodus::models::devicecredential::{DeviceAddRequest, DeviceAddResponse};
use xodus::models::soap::Envelope;

#[derive(Deserialize)]
struct Meta {
    request: RequestMeta,
}

#[derive(Deserialize)]
struct RequestMeta {
    method: String,
    url: String,
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/login-live-pairs")
}

#[test]
fn captured_login_live_pairs_parse_with_xodus_models() {
    let root = fixture_root();
    assert!(root.exists(), "missing fixture directory: {}", root.display());

    let mut fixture_dirs: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    fixture_dirs.sort();

    assert!(!fixture_dirs.is_empty(), "no fixture directories in {}", root.display());

    for dir in fixture_dirs {
        let meta: Meta = serde_json::from_str(&fs::read_to_string(dir.join("meta.json")).unwrap())
            .unwrap();
        assert_eq!(meta.request.method, "POST", "unexpected method in {}", dir.display());

        let request_xml = fs::read_to_string(dir.join("request-body.xml")).unwrap();
        let response_xml = fs::read_to_string(dir.join("response-body.xml")).unwrap();

        if meta.request.url.ends_with("/ppsecure/deviceaddcredential.srf") {
            let request: DeviceAddRequest = quick_xml::de::from_str(&request_xml)
                .unwrap_or_else(|err| panic!("{} request parse failed: {err}", dir.display()));
            if request.device_info.is_none() {
                continue;
            }
            let _: DeviceAddResponse = quick_xml::de::from_str(&response_xml)
                .unwrap_or_else(|err| panic!("{} response parse failed: {err}", dir.display()));
        } else if meta.request.url.ends_with("/RST2.srf") {
            let _: Envelope = quick_xml::de::from_str(&request_xml)
                .unwrap_or_else(|err| panic!("{} request parse failed: {err}", dir.display()));
            let _: Envelope = quick_xml::de::from_str(&response_xml)
                .unwrap_or_else(|err| panic!("{} response parse failed: {err}", dir.display()));
        } else {
            panic!(
                "unexpected login.live.com endpoint in fixture {}: {}",
                dir.display(),
                meta.request.url
            );
        }
    }
}
