use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tepra_cli::webapi::WebApi;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn server(
    tape: i64,
    disconnect: bool,
    complete: bool,
) -> (String, Arc<Mutex<Vec<(String, String)>>>) {
    server_with_override(tape, disconnect, complete, None).await
}

async fn server_with_override(
    tape: i64,
    disconnect: bool,
    complete: bool,
    override_response: Option<(&'static str, &'static str)>,
) -> (String, Arc<Mutex<Vec<(String, String)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let captured = captured.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0; 4096];
                let header_end = loop {
                    let Ok(n) = stream.read(&mut chunk).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|x| x == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&buf[..header_end]);
                let line = headers.lines().next().unwrap().to_owned();
                let len = headers
                    .lines()
                    .find_map(|s| {
                        s.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                while buf.len() < header_end + len {
                    let Ok(n) = stream.read(&mut chunk).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body = String::from_utf8_lossy(&buf[header_end..]).to_string();
                captured.lock().unwrap().push((line.clone(), body));
                if line.starts_with("POST") && disconnect {
                    return;
                }
                let json = if let Some((_, body)) =
                    override_response.filter(|(path, _)| line.contains(path))
                {
                    body.to_string()
                } else if line.starts_with("POST") {
                    r#"{"result":1,"jobid":42}"#.to_string()
                } else if line.contains("/info/") {
                    r#"{"driverName":"KING JIM SR-MK2","dpi":360,"tapeList":[{"tapeID":263}]}"#
                        .into()
                } else if line.contains("/onlinestatus/") {
                    r#"{"online":true}"#.into()
                } else if line.contains("/lwstatus/") {
                    format!(r#"{{"tapeID":{tape},"tapeKind":0,"error":0,"status":0}}"#)
                } else if line.contains("/job/progress/") {
                    format!(
                        r#"{{"dataProgress":100,"jobEnd":{complete},"canceled":false,"statusError":0}}"#
                    )
                } else {
                    r#"[{"printerName":"名前 / #"}]"#.into()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    json.len(),
                    json
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (format!("http://{address}/api/printer"), requests)
}

#[tokio::test]
async fn success_encodes_name_payload_and_waits_for_completion() {
    let (endpoint, requests) = server(263, false, true).await;
    let api = WebApi::with_endpoint(&endpoint).unwrap();
    let result = api
        .print("名前 / #", b"png bytes", "local-id", Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(result["completion_evidence"], "vendor_job_end");
    let req = requests.lock().unwrap();
    let post: Vec<_> = req
        .iter()
        .filter(|(line, _)| line.starts_with("POST"))
        .collect();
    assert_eq!(post.len(), 1);
    assert!(post[0].0.contains("%E5%90%8D%E5%89%8D%20%2F%20%23"));
    let body: serde_json::Value = serde_json::from_str(&post[0].1).unwrap();
    assert_eq!(body["printFile"]["imageFile"]["base64Str"], "cG5nIGJ5dGVz");
    assert_eq!(body["printParameter"]["copies"], 1);
    assert!(req.iter().any(|(line, _)| line.contains("jobid=42")));
}
#[tokio::test]
async fn tape_mismatch_does_not_submit() {
    let (endpoint, requests) = server(262, false, true).await;
    let error = WebApi::with_endpoint(&endpoint)
        .unwrap()
        .print("名前 / #", b"png", "id", Duration::from_secs(1))
        .await
        .unwrap_err();
    assert_eq!(error.exit_code, 5);
    assert!(
        !requests
            .lock()
            .unwrap()
            .iter()
            .any(|(line, _)| line.starts_with("POST"))
    );
}
#[tokio::test]
async fn disconnect_never_retries_post() {
    let (endpoint, requests) = server(263, true, true).await;
    let error = WebApi::with_endpoint(&endpoint)
        .unwrap()
        .print("名前 / #", b"png", "id", Duration::from_secs(1))
        .await
        .unwrap_err();
    assert_eq!(error.exit_code, 7);
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(line, _)| line.starts_with("POST"))
            .count(),
        1
    );
}
#[tokio::test]
async fn data_progress_alone_does_not_complete() {
    let (endpoint, requests) = server(263, false, false).await;
    let error = WebApi::with_endpoint(&endpoint)
        .unwrap()
        .print("名前 / #", b"png", "id", Duration::from_millis(100))
        .await
        .unwrap_err();
    assert_eq!(error.exit_code, 7);
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(line, _)| line.starts_with("POST"))
            .count(),
        1
    );
}
#[test]
fn rejects_remote_endpoint() {
    assert!(WebApi::with_endpoint("http://example.com/api/printer").is_err());
    assert!(WebApi::with_endpoint("https://localhost/api/printer").is_err());
}

#[tokio::test]
async fn rejects_dot_names_before_network_access() {
    let api = WebApi::with_endpoint("http://127.0.0.1:9/api/printer").unwrap();
    for name in [".", ".."] {
        assert_eq!(api.status(name).await.unwrap_err().exit_code, 2);
        assert_eq!(
            api.print(name, b"png", "id", Duration::from_secs(1))
                .await
                .unwrap_err()
                .exit_code,
            2
        );
    }
}

#[tokio::test]
async fn completed_jobs_report_cancellation_and_device_errors() {
    for (response, code, state) in [
        (
            r#"{"jobEnd":true,"canceled":true,"statusError":0}"#,
            "print_canceled",
            "canceled",
        ),
        (
            r#"{"jobEnd":true,"canceled":false,"statusError":33}"#,
            "printer_error",
            "failed",
        ),
    ] {
        let (endpoint, requests) =
            server_with_override(263, false, true, Some(("/job/progress/", response))).await;
        let error = WebApi::with_endpoint(&endpoint)
            .unwrap()
            .print("名前 / #", b"png", "id", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(error.exit_code, 5);
        assert_eq!(error.code, code);
        assert!(!error.retryable);
        let context = error.context.unwrap();
        assert_eq!(context["state"], state);
        assert_eq!(context["vendor_job_id"], 42);
        assert_eq!(context["progress"]["jobEnd"], true);
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(line, _)| line.starts_with("POST"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn missing_completion_details_are_unknown() {
    let (endpoint, _) = server_with_override(
        263,
        false,
        true,
        Some(("/job/progress/", r#"{"jobEnd":true,"canceled":false}"#)),
    )
    .await;
    let error = WebApi::with_endpoint(&endpoint)
        .unwrap()
        .print("名前 / #", b"png", "id", Duration::from_secs(1))
        .await
        .unwrap_err();
    assert_eq!(error.exit_code, 7);
    assert_eq!(error.context.unwrap()["vendor_job_id"], 42);
}

#[tokio::test]
async fn rejects_error_and_malformed_status_responses() {
    for (path, body) in [
        ("GET /api/printer HTTP", r#"{"errcode":12}"#),
        ("GET /api/printer HTTP", r#"[{"unexpected":"name"}]"#),
        ("/info/", r#"{"errcode":12}"#),
        ("/info/", r#"{"driverName":"KING JIM SR-MK2"}"#),
        ("/onlinestatus/", r#"{"online":"yes"}"#),
        ("/lwstatus/", r#"{"tapeID":263}"#),
        (
            "/lwstatus/",
            r#"{"tapeID":263,"tapeKind":0,"error":0,"status":0,"tapeSw":"unknown"}"#,
        ),
    ] {
        let (endpoint, requests) = server_with_override(263, false, true, Some((path, body))).await;
        let api = WebApi::with_endpoint(&endpoint).unwrap();
        let result = if path == "GET /api/printer HTTP" {
            api.devices().await
        } else {
            api.status("名前 / #").await
        };
        assert_eq!(result.unwrap_err().exit_code, 6);
        assert!(
            !requests
                .lock()
                .unwrap()
                .iter()
                .any(|(line, _)| line.starts_with("POST"))
        );
    }
}

#[tokio::test]
async fn rejects_oversized_response() {
    let body: &'static str =
        Box::leak(format!("[\"{}\"]", "x".repeat(1024 * 1024)).into_boxed_str());
    let (endpoint, _) =
        server_with_override(263, false, true, Some(("GET /api/printer HTTP", body))).await;
    let error = WebApi::with_endpoint(&endpoint)
        .unwrap()
        .devices()
        .await
        .unwrap_err();
    assert_eq!(error.exit_code, 6);
}
