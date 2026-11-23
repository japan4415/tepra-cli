//! Windows TEPRA Creator WebAPI transport. The API must be local to this host.
use crate::error::{AppError, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Client, Url};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::time::{Instant, timeout_at};

const DEFAULT_PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct WebApi {
    client: Client,
    endpoint: Url,
    preflight_timeout: Duration,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrintRequest {
    print_file: PrintFile,
    print_parameter: PrintParameter,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrintFile {
    image_file: ImageFile,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageFile {
    file_name: &'static str,
    base64_str: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrintParameter {
    copies: u32,
    tape_cut: u32,
    half_cut: u32,
    print_speed: u32,
    density: DensityParameter,
    #[serde(rename = "tapeID")]
    tape_id: u32,
    priority_cut_setting: u32,
    half_cut_separate: u32,
    margin_left_right: u32,
    display_tape_width: u32,
    error_message: ErrorMessageParameter,
    display_transfer_tape: u32,
    display_print_setting: u32,
    cut_title: u32,
    kana_zen: u32,
    display_print_preview: u32,
    stretch_image: u32,
}

#[derive(Serialize)]
struct DensityParameter {
    mode: u32,
    value: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorMessageParameter {
    mode: u32,
    file_output: u32,
    file_path: &'static str,
}

fn default_24mm_print_request(png: &[u8]) -> PrintRequest {
    PrintRequest {
        print_file: PrintFile {
            image_file: ImageFile {
                file_name: "label.png",
                base64_str: STANDARD.encode(png),
            },
        },
        print_parameter: PrintParameter {
            copies: 1,
            tape_cut: 2,
            half_cut: 1,
            print_speed: 1,
            density: DensityParameter { mode: 1, value: 0 },
            tape_id: 263,
            priority_cut_setting: 1,
            half_cut_separate: 1,
            margin_left_right: 0,
            display_tape_width: 1,
            error_message: ErrorMessageParameter {
                mode: 1,
                file_output: 0,
                file_path: "",
            },
            display_transfer_tape: 1,
            display_print_setting: 1,
            cut_title: 0,
            kana_zen: 0,
            display_print_preview: 1,
            stretch_image: 0,
        },
    }
}

fn not_submitted(mut error: AppError) -> AppError {
    let context = match error.context.take() {
        Some(Value::Object(mut map)) => {
            map.insert("state".into(), "not_submitted".into());
            map.insert("transmission_started".into(), false.into());
            Value::Object(map)
        }
        Some(other) => {
            json!({"state": "not_submitted", "transmission_started": false, "detail": other})
        }
        None => json!({"state": "not_submitted", "transmission_started": false}),
    };
    error.context = Some(context);
    error
}

// The SDK converts these WebAPI fields using Boolean(...). Accept only
// explicit booleans and 0/1, never arbitrary truthy values.
fn bool_field(value: Option<&Value>) -> Option<bool> {
    value.and_then(|v| {
        v.as_bool().or_else(|| match v.as_u64() {
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => None,
        })
    })
}

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

async fn read_json(mut response: reqwest::Response) -> std::result::Result<Value, String> {
    let declared_len = response.content_length();
    if declared_len.is_some_and(|n| n > MAX_RESPONSE_BYTES as u64) {
        return Err("WebAPI response exceeds size limit".into());
    }
    let capacity = declared_len.map_or(0, |n| n as usize);
    let mut bytes = Vec::with_capacity(capacity);
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
            return Err("WebAPI response exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

impl WebApi {
    pub fn new() -> Result<Self> {
        Self::with_endpoint("http://localhost:29108/api/printer")
    }

    /// Alternate loopback endpoint, intended for local integration tests.
    pub fn with_endpoint(endpoint: &str) -> Result<Self> {
        let endpoint =
            Url::parse(endpoint).map_err(|_| AppError::invalid("invalid WebAPI endpoint"))?;
        let host_ok = endpoint.host_str().is_some_and(|h| {
            h.eq_ignore_ascii_case("localhost")
                || h.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if endpoint.scheme() != "http"
            || !host_ok
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(AppError::invalid(
                "WebAPI endpoint must be loopback HTTP without credentials, query or fragment",
            ));
        }
        // Do not set a client-wide timeout here: preflight GETs use
        // `preflight_timeout`, while print submission and completion polling
        // follow the caller's `--timeout-seconds` deadline.
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|e| AppError::unsupported(format!("failed to create HTTP client: {e}")))?;
        Ok(Self {
            client,
            endpoint,
            preflight_timeout: DEFAULT_PREFLIGHT_TIMEOUT,
        })
    }

    /// Override the per-request preflight timeout (default: 5 seconds).
    pub fn with_preflight_timeout(mut self, timeout: Duration) -> Self {
        self.preflight_timeout = timeout;
        self
    }

    fn url(&self, segments: &[&str]) -> Result<Url> {
        if segments
            .iter()
            .any(|segment| matches!(*segment, "." | ".."))
        {
            return Err(AppError::invalid("A printer name cannot be '.' or '..'"));
        }
        let mut url = self.endpoint.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| AppError::invalid("invalid WebAPI endpoint path"))?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url)
    }
    async fn get(&self, segments: &[&str]) -> Result<Value> {
        let deadline = Instant::now() + self.preflight_timeout;
        let send = self.client.get(self.url(segments)?).send();
        let response = timeout_at(deadline, send)
            .await
            .map_err(|_| {
                AppError::new(
                    "webapi_connection",
                    "WebAPI request timed out; check that the Windows communication module is installed and running",
                    4,
                )
            })?
            .map_err(|e| {
                AppError::new(
                    "webapi_connection",
                    format!(
                        "WebAPI connection failed; check that the Windows communication module is installed and running: {e}"
                    ),
                    4,
                )
            })?;
        if !response.status().is_success() {
            return Err(AppError::unsupported(format!(
                "WebAPI returned HTTP {}",
                response.status()
            )));
        }
        let value = timeout_at(deadline, read_json(response))
            .await
            .map_err(|_| {
                AppError::new(
                    "webapi_connection",
                    "WebAPI response timed out; check that the Windows communication module is installed and running",
                    4,
                )
            })?
            .map_err(|e| AppError::unsupported(format!("invalid WebAPI response: {e}")))?;
        if let Some(errcode) = value.get("errcode") {
            return Err(AppError::unsupported(format!(
                "WebAPI returned error response (errcode={errcode})"
            ))
            .with_context(json!({"errcode": errcode})));
        }
        Ok(value)
    }
    pub async fn devices(&self) -> Result<Value> {
        let value = self.get(&[]).await?;
        if !value.as_array().is_some_and(|list| {
            list.iter().all(|p| {
                p.get("printerName")
                    .and_then(Value::as_str)
                    .is_some_and(|n| !n.trim().is_empty())
            })
        }) {
            return Err(AppError::unsupported("invalid printer list"));
        }
        Ok(value)
    }
    pub async fn status(&self, printer: &str) -> Result<Value> {
        if printer.trim().is_empty() || matches!(printer, "." | "..") {
            return Err(AppError::invalid("printer name is required"));
        }
        let info = self.get(&["info", printer]).await?;
        if info.get("driverName").and_then(Value::as_str).is_none()
            || info.get("dpi").and_then(Value::as_u64).is_none()
            || !info
                .get("tapeList")
                .and_then(Value::as_array)
                .is_some_and(|list| {
                    list.iter()
                        .all(|t| t.get("tapeID").and_then(Value::as_u64).is_some())
                })
        {
            return Err(AppError::unsupported("invalid printer information"));
        }
        let online = self.get(&["onlinestatus", printer]).await?;
        if bool_field(online.get("online")).is_none() {
            return Err(AppError::unsupported("invalid online status"));
        }
        let device = self.get(&["lwstatus", printer]).await?;
        if device.get("error").and_then(Value::as_i64).is_none()
            || device.get("status").and_then(Value::as_i64).is_none()
            || device
                .get("tapeID")
                .is_some_and(|v| !v.is_null() && v.as_u64().is_none())
            || device
                .get("tapeKind")
                .is_some_and(|v| !v.is_null() && v.as_i64().is_none())
            || device
                .get("brTapeKind")
                .is_some_and(|v| !v.is_null() && v.as_u64().is_none())
            || device
                .get("tapeSw")
                .is_some_and(|v| !v.is_null() && v.as_u64().is_none())
        {
            return Err(AppError::unsupported("invalid printer status"));
        }
        Ok(json!({"info":info,"online":online,"device":device}))
    }
    pub async fn print(
        &self,
        printer: &str,
        png: &[u8],
        job_id: &str,
        timeout: Duration,
    ) -> Result<Value> {
        let submitted = AtomicBool::new(false);
        self.print_with_submission_flag(printer, png, job_id, timeout, &submitted)
            .await
    }

    pub async fn print_with_submission_flag(
        &self,
        printer: &str,
        png: &[u8],
        job_id: &str,
        timeout: Duration,
        transmission_started: &AtomicBool,
    ) -> Result<Value> {
        if printer.trim().is_empty()
            || matches!(printer, "." | "..")
            || job_id.trim().is_empty()
            || png.is_empty()
            || timeout.is_zero()
        {
            return Err(not_submitted(AppError::invalid(
                "printer, job ID, PNG and positive timeout are required",
            )));
        }
        let devices = self.devices().await.map_err(not_submitted)?;
        let list = devices
            .as_array()
            .ok_or_else(|| not_submitted(AppError::unsupported("invalid printer list")))?;
        if list
            .iter()
            .filter(|p| p.get("printerName").and_then(Value::as_str) == Some(printer))
            .count()
            != 1
        {
            return Err(not_submitted(AppError::new(
                "printer_not_found",
                "printer not found or ambiguous",
                3,
            )));
        }
        let status = self.status(printer).await.map_err(not_submitted)?;
        let info = &status["info"];
        let online = &status["online"];
        let device = &status["device"];
        if info.get("driverName").and_then(Value::as_str) != Some("KING JIM SR-MK2")
            || info.get("dpi").and_then(Value::as_u64) != Some(360)
        {
            return Err(not_submitted(AppError::new(
                "printer_mismatch",
                "printer model or resolution is not SR-MK2 at 360 dpi",
                3,
            )));
        }
        if !info
            .get("tapeList")
            .and_then(Value::as_array)
            .is_some_and(|a| {
                a.iter()
                    .any(|t| t.get("tapeID").and_then(Value::as_u64) == Some(263))
            })
        {
            return Err(not_submitted(AppError::unsupported(
                "24 mm tape support could not be confirmed",
            )));
        }
        match bool_field(online.get("online")) {
            Some(true) => (),
            Some(false) => {
                return Err(not_submitted(AppError::new(
                    "printer_offline",
                    "printer is offline",
                    4,
                )));
            }
            None => {
                return Err(not_submitted(AppError::unsupported(
                    "online state is unknown",
                )));
            }
        }
        if device.get("tapeID").and_then(Value::as_u64) != Some(263)
            || device.get("tapeKind").and_then(Value::as_i64) != Some(0)
            || device
                .get("brTapeKind")
                .and_then(Value::as_u64)
                .is_some_and(|v| v != 0)
            || device
                .get("tapeSw")
                .and_then(Value::as_u64)
                .is_some_and(|v| v & 0x30 != 0)
        {
            return Err(not_submitted(AppError::new(
                "tape_mismatch",
                "normal 24 mm tape could not be confirmed",
                5,
            )));
        }
        match device.get("error").and_then(Value::as_i64) {
            Some(0) => (),
            Some(_) => {
                return Err(not_submitted(AppError::new(
                    "printer_error",
                    "printer reports an error",
                    5,
                )));
            }
            None => {
                return Err(not_submitted(AppError::unsupported(
                    "printer error state is unknown",
                )));
            }
        }
        match device.get("status").and_then(Value::as_i64) {
            Some(0x12 | 0x13) => {
                return Err(not_submitted(AppError::new(
                    "printer_busy",
                    "printer is busy or updating",
                    4,
                )));
            }
            // The SDK maps 0x12 and 0x13 to busy/updating; its default
            // branch leaves the separately checked device error unchanged.
            Some(0..=255) => (),
            _ => {
                return Err(not_submitted(AppError::unsupported(
                    "printer readiness is unknown",
                )));
            }
        }
        let payload = default_24mm_print_request(png);
        let print_url = self.url(&["print", printer]).map_err(not_submitted)?;
        let context = |vendor: Option<&Value>| json!({"job_id":job_id,"device_id":printer,"vendor_job_id":vendor,"state":"unknown","transmission_started":true});
        let deadline = Instant::now() + timeout;
        transmission_started.store(true, Ordering::Release);
        let send = self.client.post(print_url).json(&payload).send();
        let response = timeout_at(deadline, send)
            .await
            .map_err(|_| {
                AppError::unknown("print submission timed out").with_context(context(None))
            })?
            .map_err(|e| {
                AppError::unknown(format!("print submission failed: {e}"))
                    .with_context(context(None))
            })?;
        if !response.status().is_success() {
            return Err(AppError::unknown(format!(
                "print submission returned HTTP {}",
                response.status()
            ))
            .with_context(context(None)));
        }
        let body: Value = timeout_at(deadline, read_json(response))
            .await
            .map_err(|_| AppError::unknown("print response timed out").with_context(context(None)))?
            .map_err(|e| {
                AppError::unknown(format!("invalid print response: {e}"))
                    .with_context(context(None))
            })?;
        let vendor = body
            .get("jobid")
            .filter(|v| v.as_u64().is_some())
            .ok_or_else(|| {
                AppError::unknown("print response lacks a job ID").with_context(context(None))
            })?;
        if body.get("result").and_then(Value::as_i64) != Some(1) {
            return Err(AppError::unknown("print submission was not confirmed")
                .with_context(context(Some(vendor))));
        }
        let mut url = self.url(&["job", "progress", printer])?;
        url.query_pairs_mut()
            .append_pair("jobid", &vendor.to_string());
        loop {
            let response = timeout_at(deadline, self.client.get(url.clone()).send())
                .await
                .map_err(|_| {
                    AppError::unknown("print completion timed out")
                        .with_context(context(Some(vendor)))
                })?
                .map_err(|e| {
                    AppError::unknown(format!("progress request failed: {e}"))
                        .with_context(context(Some(vendor)))
                })?;
            if !response.status().is_success() {
                return Err(AppError::unknown(format!(
                    "progress returned HTTP {}",
                    response.status()
                ))
                .with_context(context(Some(vendor))));
            }
            let progress: Value = timeout_at(deadline, read_json(response))
                .await
                .map_err(|_| {
                    AppError::unknown("progress response timed out")
                        .with_context(context(Some(vendor)))
                })?
                .map_err(|e| {
                    AppError::unknown(format!("invalid progress response: {e}"))
                        .with_context(context(Some(vendor)))
                })?;
            match bool_field(progress.get("jobEnd")) {
                Some(true)
                    if bool_field(progress.get("canceled")) == Some(false)
                        && progress.get("statusError").and_then(Value::as_i64) == Some(0) =>
                {
                    return Ok(
                        json!({"job_id":job_id,"device_id":printer,"vendor_job_id":vendor,"state":"completed","completion_evidence":"vendor_job_end","progress":progress}),
                    );
                }
                Some(true) => match (bool_field(progress.get("canceled")), progress.get("statusError").and_then(Value::as_i64)) {
                    (Some(true), _) => return Err(AppError::new("print_canceled", "print job was canceled", 5).with_context(json!({"job_id":job_id,"device_id":printer,"vendor_job_id":vendor,"state":"canceled","progress":progress}))),
                    (_, Some(error)) if error != 0 => return Err(AppError::new("printer_error", "print job ended with a device error", 5).with_context(json!({"job_id":job_id,"device_id":printer,"vendor_job_id":vendor,"state":"failed","progress":progress}))),
                    _ => return Err(AppError::unknown("job completion details are missing").with_context(context(Some(vendor)))),
                },
                Some(false) => (),
                None => {
                    return Err(AppError::unknown("job completion state is missing")
                        .with_context(context(Some(vendor))));
                }
            }
            timeout_at(deadline, tokio::time::sleep(Duration::from_millis(500)))
                .await
                .map_err(|_| {
                    AppError::unknown("print completion timed out")
                        .with_context(context(Some(vendor)))
                })?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_print_payload_matches_vendor_schema() {
        let actual = serde_json::to_value(default_24mm_print_request(b"png bytes")).unwrap();
        let expected = json!({
            "printFile": {
                "imageFile": {
                    "fileName": "label.png",
                    "base64Str": "cG5nIGJ5dGVz"
                }
            },
            "printParameter": {
                "copies": 1,
                "tapeCut": 2,
                "halfCut": 1,
                "printSpeed": 1,
                "density": {"mode": 1, "value": 0},
                "tapeID": 263,
                "priorityCutSetting": 1,
                "halfCutSeparate": 1,
                "marginLeftRight": 0,
                "displayTapeWidth": 1,
                "errorMessage": {"mode": 1, "fileOutput": 0, "filePath": ""},
                "displayTransferTape": 1,
                "displayPrintSetting": 1,
                "cutTitle": 0,
                "kanaZen": 0,
                "displayPrintPreview": 1,
                "stretchImage": 0
            }
        });
        assert_eq!(actual, expected);
    }
}
