#!/usr/bin/env python3

import argparse
import base64
import json
import sys
from pathlib import Path
from urllib.parse import urlparse
from xml.dom import minidom


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Filter a HAR file down to requests for a specific host"
    )
    parser.add_argument("har", help="Path to the HAR file")
    parser.add_argument(
        "--host",
        default="login.live.com",
        help="Host to match (default: login.live.com)",
    )
    parser.add_argument(
        "--output",
        help="Write the filtered HAR to this file instead of stdout",
    )
    parser.add_argument(
        "--keep-connect",
        action="store_true",
        help="Keep CONNECT requests instead of dropping them",
    )
    parser.add_argument(
        "--extract-dir",
        help="Write one folder per matched request/response pair into this directory",
    )
    return parser.parse_args()


def host_matches(url: str, expected_host: str) -> bool:
    parsed = urlparse(url)
    hostname = parsed.hostname
    if hostname is None:
        return expected_host in url
    return hostname == expected_host


def filter_har(har_path: Path, expected_host: str, keep_connect: bool) -> tuple[dict, int]:
    with har_path.open(encoding="utf-8-sig") as handle:
        har = json.load(handle)

    log = har.get("log", {})
    entries = log.get("entries", [])
    filtered_entries = []

    for entry in entries:
        request = entry.get("request")
        if not request:
            continue
        if not keep_connect and request.get("method") == "CONNECT":
            continue
        url = request.get("url", "")
        if not host_matches(url, expected_host):
            continue
        filtered_entries.append(entry)

    filtered = dict(har)
    filtered["log"] = dict(log)
    filtered["log"]["entries"] = filtered_entries
    return filtered, len(filtered_entries)


def body_bytes(content: dict | None) -> bytes:
    if not content:
        return b""
    text = content.get("text")
    if text is None:
        return b""
    if content.get("encoding") == "base64":
        return base64.b64decode(text)
    return text.encode("utf-8")


def request_body_bytes(request: dict) -> bytes:
    post_data = request.get("postData")
    if not post_data:
        return b""
    text = post_data.get("text")
    if text is None:
        return b""
    return text.encode("utf-8")


def pretty_xml_bytes(data: bytes) -> bytes | None:
    if not data:
        return None
    try:
        parsed = minidom.parseString(data)
    except Exception:
        return None
    pretty = parsed.toprettyxml(indent="  ", encoding="utf-8")
    return pretty


def write_pair_folders(entries: list[dict], root: Path) -> None:
    root.mkdir(parents=True, exist_ok=True)

    for index, entry in enumerate(entries, start=1):
        pair_dir = root / f"{index:04d}"
        pair_dir.mkdir(exist_ok=True)

        request = entry.get("request", {})
        response = entry.get("response", {})

        meta = {
            "startedDateTime": entry.get("startedDateTime"),
            "time": entry.get("time"),
            "serverIPAddress": entry.get("serverIPAddress"),
            "connection": entry.get("connection"),
            "request": {
                "method": request.get("method"),
                "url": request.get("url"),
                "httpVersion": request.get("httpVersion"),
                "headers": request.get("headers", []),
                "queryString": request.get("queryString", []),
                "cookies": request.get("cookies", []),
                "postData": {
                    k: v for k, v in (request.get("postData") or {}).items() if k != "text"
                },
            },
            "response": {
                "status": response.get("status"),
                "statusText": response.get("statusText"),
                "httpVersion": response.get("httpVersion"),
                "headers": response.get("headers", []),
                "cookies": response.get("cookies", []),
                "redirectURL": response.get("redirectURL"),
                "content": {
                    k: v for k, v in (response.get("content") or {}).items() if k != "text"
                },
            },
        }
        (pair_dir / "meta.json").write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")

        request_body = request_body_bytes(request)
        if request_body:
            (pair_dir / "request-body.bin").write_bytes(request_body)
            pretty_request = pretty_xml_bytes(request_body)
            if pretty_request is not None:
                (pair_dir / "request-body.xml").write_bytes(pretty_request)

        response_body = body_bytes(response.get("content"))
        if response_body:
            (pair_dir / "response-body.bin").write_bytes(response_body)
            pretty_response = pretty_xml_bytes(response_body)
            if pretty_response is not None:
                (pair_dir / "response-body.xml").write_bytes(pretty_response)


def main() -> int:
    args = parse_args()
    filtered_har, count = filter_har(Path(args.har), args.host, args.keep_connect)
    entries = filtered_har.get("log", {}).get("entries", [])

    if args.extract_dir:
        write_pair_folders(entries, Path(args.extract_dir))

    output = json.dumps(filtered_har, indent=2)

    if args.output:
        Path(args.output).write_text(output + "\n", encoding="utf-8")
    else:
        sys.stdout.write(output + "\n")

    print(
        f"Extracted {count} request(s) for host {args.host}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
