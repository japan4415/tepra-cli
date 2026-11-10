//! Experimental SR-MK2 USB protocol. Restricted to the observed 24 mm profile.
use crate::error::{AppError, Result};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_LENGTH: u32 = 1417;
const HEAD: u32 = 256;
const EXPECTED_TAPE_CODE: Option<u8> = Some(5);
// TEPRA LINK 2 image-data API default: truncate(360 * 1 / 25.4), in dots.
// Physical leading/trailing feed is not implied by this parameter alone.
const PAGE_MARGIN: Option<u16> = Some(14);

/// Offline validation for the experimental USB profile; never opens a device.
pub fn validate(png: &[u8]) -> Result<()> {
    decode(png).map(|_| ())
}

struct Diagnostics {
    phase: &'static str,
    transmission_started: bool,
    resolved_device_id: Option<String>,
    raw_status: Option<Vec<u8>>,
    transitions: Vec<u8>,
    transitions_truncated: bool,
    #[cfg(any(target_os = "macos", target_os = "linux", test))]
    last_st: Option<u8>,
}
impl Default for Diagnostics {
    fn default() -> Self {
        Self {
            phase: "preflight",
            transmission_started: false,
            resolved_device_id: None,
            raw_status: None,
            transitions: Vec::new(),
            transitions_truncated: false,
            #[cfg(any(target_os = "macos", target_os = "linux", test))]
            last_st: None,
        }
    }
}
impl Diagnostics {
    #[cfg(any(target_os = "macos", target_os = "linux", test))]
    fn record(&mut self, data: &[u8]) {
        self.raw_status = Some(data.to_vec());
        if let Some(&st) = data.get(1)
            && self.last_st != Some(st)
        {
            if self.transitions.len() < 16 {
                self.transitions.push(st);
            } else {
                self.transitions_truncated = true;
            }
            self.last_st = Some(st);
        }
    }
    fn attach(&self, error: AppError) -> AppError {
        error.with_context(self.snapshot())
    }
    fn snapshot(&self) -> Value {
        serde_json::json!({"phase":self.phase,"transmission_started":self.transmission_started,"resolved_device_id":self.resolved_device_id,"raw_status":self.raw_status,"observed_st_transitions":self.transitions,"observed_st_transitions_truncated":self.transitions_truncated})
    }
}

/// Shareable progress for cancellation and diagnostics of a single print attempt.
#[derive(Clone, Default)]
pub struct PrintProgress(Arc<Mutex<Diagnostics>>);
impl PrintProgress {
    fn access<T>(&self, f: impl FnOnce(&mut Diagnostics) -> T) -> T {
        f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
    pub fn snapshot(&self) -> Value {
        self.access(|d| d.snapshot())
    }
    pub fn interrupted(&self) -> AppError {
        self.access(|d| {
            let mut error = if d.transmission_started {
                AppError::unknown(
                    "Print interrupted after transmission began; physical result may be partial",
                )
            } else {
                AppError::new(
                    "print_interrupted",
                    "Print interrupted before submission",
                    4,
                )
            };
            let mut context = d.snapshot();
            if !d.transmission_started {
                context["state"] = "not_submitted".into();
            }
            error.context = Some(context);
            error
        })
    }
    fn attach(&self, error: AppError) -> AppError {
        self.access(|d| d.attach(error))
    }
    #[cfg(any(target_os = "macos", target_os = "linux", test))]
    fn phase(&self, phase: &'static str) {
        self.access(|d| d.phase = phase);
    }
    #[cfg(any(target_os = "macos", target_os = "linux", test))]
    fn record(&self, data: &[u8]) {
        self.access(|d| d.record(data));
    }
    #[cfg(test)]
    fn started(&self) {
        self.access(|d| d.transmission_started = true);
    }
    #[cfg(any(target_os = "macos", target_os = "linux", test))]
    fn start_before(&self, deadline: tokio::time::Instant) -> Result<()> {
        self.access(|d| {
            if tokio::time::Instant::now() >= deadline {
                return Err(if d.transmission_started {
                    AppError::unknown(
                        "USB print timed out after transmission; physical result may be partial",
                    )
                } else {
                    AppError::new("device_timeout", "USB print timed out before submission", 4)
                });
            }
            d.transmission_started = true;
            Ok(())
        })
    }
    #[cfg(any(target_os = "macos", target_os = "linux", test))]
    fn set_resolved_device_id(&self, id: String) {
        self.access(|d| d.resolved_device_id = Some(id));
    }
    #[cfg(any(target_os = "macos", target_os = "linux", test))]
    fn timeout_error(&self) -> AppError {
        self.access(|d| {
            if d.transmission_started {
                AppError::unknown(
                    "USB print timed out after transmission; physical result may be partial",
                )
            } else {
                AppError::new("device_timeout", "USB print timed out before submission", 4)
            }
        })
    }
}

#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
struct Raster {
    width: u32,
    pixels: Vec<u8>,
}
fn decode(png: &[u8]) -> Result<Raster> {
    if png.len() > crate::render::MAX_FILE_BYTES as usize {
        return Err(AppError::invalid("raster PNG exceeds size limit"));
    }
    let decoder = png::Decoder::new(std::io::BufReader::new(std::io::Cursor::new(png)));
    let mut reader = decoder
        .read_info()
        .map_err(|e| AppError::invalid(format!("invalid raster PNG: {e}")))?;
    let info = reader.info();
    let width = info.width;
    if width == 0
        || width > MAX_LENGTH
        || info.height != HEAD
        || info.color_type != png::ColorType::Grayscale
        || info.bit_depth != png::BitDepth::Eight
        || info.trns.is_some()
        || info.animation_control.is_some()
        || info.frame_control.is_some()
        || !matches!(
            info.pixel_dims,
            Some(png::PixelDimensions {
                xppu: 14173,
                yppu: 14173,
                unit: png::Unit::Meter
            })
        )
    {
        return Err(AppError::invalid(
            "unsupported raster dimensions or PNG format",
        ));
    }
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| AppError::invalid("invalid raster buffer"))?;
    if size != (width * HEAD) as usize {
        return Err(AppError::invalid("invalid raster buffer size"));
    }
    let mut pixels = vec![0; size];
    reader
        .next_frame(&mut pixels)
        .map_err(|e| AppError::invalid(format!("invalid raster PNG: {e}")))?;
    if pixels.iter().any(|&p| p != 0 && p != 255) {
        return Err(AppError::invalid("raster is not binary"));
    }
    Ok(Raster { width, pixels })
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn command(op: u8, data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1b, 0x7b, (data.len() + 3) as u8, op];
    out.extend_from_slice(data);
    out.push(data.iter().fold(op, |sum, b| sum.wrapping_add(*b)));
    out.push(0x7d);
    out
}
#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn header(width: u32, margin: u16) -> Vec<u8> {
    // The image-data API clamps its margin to the source image width.
    let margin = u32::from(margin).min(width) as u16;
    let mut out = Vec::new();
    for (op, data) in [
        (b'@', &[][..]),
        (b'{', &[0, 0, b'S', b'T'][..]),
        // Official UI Tape Cut ON -> TapeCut=0, HalfCut=0.
        (b'C', &[1, 1, 1, 1][..]),
        (b'D', &[5][..]),
        (b'G', &[][..]),
    ] {
        out.extend(command(op, data));
    }
    out.extend(command(b'L', &width.to_le_bytes()));
    out.extend(command(b'T', &margin.to_le_bytes()));
    out
}
#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn rows(r: &Raster, start: u32, end: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((end - start) as usize * 40);
    for x in start..end {
        out.extend_from_slice(&[0x1b, 0x2e, 0, 0, 0, 1, 0, 1]);
        for y in (0..HEAD).step_by(8) {
            let mut b = 0;
            for bit in 0..8 {
                // Official Dotbydot/CoreGraphics transform maps source y to
                // HEAD - 1 - y on the head. Keep previews in source orientation.
                let source_y = HEAD - 1 - (y + bit);
                if r.pixels[(source_y * r.width + x) as usize] == 0 {
                    b |= 0x80 >> bit;
                }
            }
            out.push(b);
        }
    }
    out
}
#[derive(Default)]
#[cfg(any(target_os = "macos", target_os = "linux", test))]
struct CompletionState {
    busy_seen: bool,
    transitions: Vec<u8>,
    transitions_truncated: bool,
}
#[cfg(any(target_os = "macos", target_os = "linux", test))]
impl CompletionState {
    fn observe(&mut self, data: &[u8]) -> Result<bool> {
        if data.len() != 22 || data[0] != 22 {
            return Err(AppError::unknown(
                "Unexpected status after transmission; physical result may be partial",
            ));
        }
        if data[2] != 0 {
            return Err(AppError::new(
                "device_error",
                format!(
                    "Printer reported error {} ({}); physical result may be partial",
                    data[2],
                    crate::usb_status::error_name(data[2])
                ),
                5,
            ));
        }
        if self.transitions.last() != Some(&data[1]) {
            if self.transitions.len() < 16 {
                self.transitions.push(data[1]);
            } else {
                self.transitions_truncated = true;
            }
        }
        match data[1] {
            1 | 2 => {
                self.busy_seen = true;
                Ok(false)
            }
            3..=5 => Ok(false),
            0 => Ok(self.busy_seen),
            _ => Err(AppError::unknown(
                "Unexpected printer state; physical result may be partial",
            )),
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn validate_preflight(data: &[u8], tape: u8) -> Result<()> {
    if data.len() != 22 || data[0] != 22 {
        return Err(AppError::unsupported("Unexpected SR-MK2 status"));
    }
    if data[2] != 0 {
        return Err(AppError::new(
            "device_error",
            format!(
                "Printer reported error {} ({})",
                data[2],
                crate::usb_status::error_name(data[2])
            ),
            5,
        ));
    }
    match data[1] {
        0 => (),
        1..=5 => return Err(AppError::new("printer_busy", "Printer is not idle", 4)),
        _ => {
            return Err(AppError::unsupported(format!(
                "Unexpected printer state {}",
                data[1]
            )));
        }
    }
    if data[6] != tape {
        return Err(AppError::new("tape_mismatch", "Expected 24 mm tape", 5));
    }
    if data[7] != 0 || data[8] != 5 || data[9] != 0 || data[10] != 0 || data[11] != 1 {
        return Err(AppError::unsupported(
            "Tape does not match the observed 24 mm cartridge profile",
        ));
    }
    Ok(())
}

pub async fn print(device_id: &str, png: &[u8], job_id: &str, timeout: Duration) -> Result<Value> {
    print_with_progress(device_id, png, job_id, timeout, PrintProgress::default()).await
}

pub async fn print_with_progress(
    device_id: &str,
    png: &[u8],
    job_id: &str,
    timeout: Duration,
    progress: PrintProgress,
) -> Result<Value> {
    let raster = decode(png).map_err(|e| progress.attach(e))?;
    let (Some(tape), Some(margin)) = (EXPECTED_TAPE_CODE, PAGE_MARGIN) else {
        return Err(progress.attach(AppError::unsupported(
            "Experimental USB printing is disabled until the 24 mm tape code and page margin are verified",
        )));
    };
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        usb_print(device_id, &raster, job_id, timeout, tape, margin, progress).await
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (device_id, raster, job_id, timeout, tape, margin);
        Err(progress.attach(AppError::unsupported(
            "USB printing is supported on macOS/Linux only",
        )))
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn usb_print(
    device_id: &str,
    raster: &Raster,
    job_id: &str,
    timeout: Duration,
    tape: u8,
    margin: u16,
    progress: PrintProgress,
) -> Result<Value> {
    use nusb::{
        descriptors::TransferType,
        transfer::{Bulk, ControlIn, ControlType, Out, Recipient},
    };
    use std::future::IntoFuture;
    let deadline = tokio::time::Instant::now() + timeout;
    let unavailable = |e: String| AppError::new("device_unavailable", e, 4);
    let preflight = async {
        let devices: Vec<_> = nusb::list_devices()
            .await
            .map_err(|e| unavailable(e.to_string()))?
            .collect();
        let info = crate::discovery::resolve_usb_device(device_id, &devices)?;
        progress.set_resolved_device_id(format!("usb:{}:{}", info.bus_id(), info.device_address()));
        if info.vendor_id() != 0x0d8a || info.product_id() != 0x0103 {
            return Err(AppError::new(
                "printer_mismatch",
                "Expected SR-MK2 USB 0d8a:0103",
                3,
            ));
        }
        let device = info.open().await.map_err(|e| unavailable(e.to_string()))?;
        let config = device
            .active_configuration()
            .map_err(|e| unavailable(e.to_string()))?;
        if config.configuration_value() != 1
            || !config.interface_alt_settings().any(|i| {
                i.interface_number() == 0
                    && i.alternate_setting() == 0
                    && i.class() == 7
                    && i.subclass() == 1
                    && i.protocol() == 2
                    && [0x81, 0x02].iter().all(|addr| {
                        i.endpoints().any(|e| {
                            e.address() == *addr
                                && e.transfer_type() == TransferType::Bulk
                                && e.max_packet_size() == 64
                        })
                    })
            })
        {
            return Err(AppError::unsupported(
                "Unexpected SR-MK2 USB interface configuration",
            ));
        }
        let data = device
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Interface,
                    request: 1,
                    value: 0,
                    index: 0,
                    length: 22,
                },
                Duration::from_secs(1),
            )
            .into_future()
            .await
            .map_err(|e| unavailable(e.to_string()))?;
        progress.record(&data);
        validate_preflight(&data, tape)?;
        let interface = device
            .claim_interface(0)
            .await
            .map_err(|e| unavailable(e.to_string()))?;
        Ok((device, interface))
    };
    let preflight_result = tokio::time::timeout_at(deadline, preflight).await;
    let (device, interface) = preflight_result
        .map_err(|_| {
            progress.attach(AppError::new(
                "device_timeout",
                "USB preflight timed out",
                4,
            ))
        })?
        .map_err(|e| progress.attach(e))?;
    let ep = interface
        .endpoint::<Bulk, Out>(0x02)
        .map_err(|e| progress.attach(unavailable(e.to_string())))?;
    let mut transport = UsbTransport { device, ep };
    execute_job(
        &mut transport,
        raster,
        job_id,
        device_id,
        margin,
        &progress,
        deadline,
    )
    .await
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
trait Transport {
    async fn send(
        &mut self,
        data: Vec<u8>,
        progress: &PrintProgress,
        deadline: tokio::time::Instant,
    ) -> Result<()>;
    async fn status(&mut self) -> Result<Vec<u8>>;
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn check_transfer(expected: usize, actual: usize, ok: bool) -> Result<()> {
    if !ok || actual != expected {
        return Err(AppError::unknown(
            "USB transfer failed or was partial; physical result may be partial",
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
async fn execute_job<T: Transport>(
    transport: &mut T,
    raster: &Raster,
    job_id: &str,
    device_id: &str,
    margin: u16,
    progress: &PrintProgress,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    let outcome = tokio::time::timeout_at(
        deadline,
        run_job(
            transport, raster, job_id, device_id, margin, progress, deadline,
        ),
    )
    .await;
    outcome
        .map_err(|_| progress.attach(progress.timeout_error()))?
        .map_err(|e| progress.attach(e))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct UsbTransport {
    device: nusb::Device,
    ep: nusb::Endpoint<nusb::transfer::Bulk, nusb::transfer::Out>,
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Transport for UsbTransport {
    async fn send(
        &mut self,
        data: Vec<u8>,
        progress: &PrintProgress,
        deadline: tokio::time::Instant,
    ) -> Result<()> {
        let len = data.len();
        let data = data.into();
        progress.start_before(deadline)?;
        self.ep.submit(data);
        let completion = self.ep.next_complete().await;
        check_transfer(len, completion.actual_len, completion.status.is_ok())
    }
    async fn status(&mut self) -> Result<Vec<u8>> {
        use nusb::transfer::{ControlIn, ControlType, Recipient};
        use std::future::IntoFuture;
        self.device
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Interface,
                    request: 1,
                    value: 0,
                    index: 0,
                    length: 22,
                },
                Duration::from_secs(1),
            )
            .into_future()
            .await
            .map_err(|e| {
                AppError::unknown(format!(
                    "Status read failed after transmission: {e}; physical result may be partial"
                ))
            })
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
async fn run_job<T: Transport>(
    transport: &mut T,
    raster: &Raster,
    job_id: &str,
    device_id: &str,
    margin: u16,
    progress: &PrintProgress,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    progress.phase("header");
    transport
        .send(header(raster.width, margin), progress, deadline)
        .await?;
    for start in (0..raster.width).step_by(1024) {
        progress.phase("raster");
        transport
            .send(
                rows(raster, start, (start + 1024).min(raster.width)),
                progress,
                deadline,
            )
            .await?;
        let data = transport.status().await?;
        progress.record(&data);
        if tokio::time::Instant::now() >= deadline {
            return Err(progress.timeout_error());
        }
        CompletionState::default().observe(&data)?;
    }
    progress.phase("form_feed");
    transport.send(vec![0x0c], progress, deadline).await?;
    progress.phase("final_reset");
    transport
        .send(command(b'@', &[]), progress, deadline)
        .await?;
    progress.phase("wait_completion");
    let mut state = CompletionState::default();
    let final_status = loop {
        let data = transport.status().await?;
        progress.record(&data);
        if tokio::time::Instant::now() >= deadline {
            return Err(progress.timeout_error());
        }
        if state.observe(&data)? {
            break data;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    Ok(
        serde_json::json!({"job_id":job_id, "device_id":device_id, "resolved_device_id":progress.snapshot()["resolved_device_id"], "state":"completed", "completion_evidence":"usb_busy_then_idle", "final_status":{"raw_bytes": final_status}, "observed_st_transitions":state.transitions, "observed_st_transitions_truncated":state.transitions_truncated}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    #[derive(Default)]
    struct Mock {
        writes: Vec<Vec<u8>>,
        statuses: VecDeque<Result<Vec<u8>>>,
        fail_send_at: Option<usize>,
        pending_send_at: Option<usize>,
    }
    impl Transport for Mock {
        async fn send(
            &mut self,
            data: Vec<u8>,
            progress: &PrintProgress,
            deadline: tokio::time::Instant,
        ) -> Result<()> {
            progress.start_before(deadline)?;
            self.writes.push(data);
            if self.pending_send_at == Some(self.writes.len()) {
                std::future::pending::<()>().await;
            }
            if self.fail_send_at == Some(self.writes.len()) {
                return check_transfer(self.writes.last().unwrap().len(), 0, true);
            }
            Ok(())
        }
        async fn status(&mut self) -> Result<Vec<u8>> {
            match self.statuses.pop_front() {
                Some(data) => data,
                None => std::future::pending().await,
            }
        }
    }
    fn status(st: u8, er: u8) -> Result<Vec<u8>> {
        let mut data = vec![0; 22];
        data[0] = 22;
        data[1] = st;
        data[2] = er;
        Ok(data)
    }
    fn test_raster(width: u32) -> Raster {
        Raster {
            width,
            pixels: vec![255; (width * HEAD) as usize],
        }
    }
    async fn run_mock(
        mock: &mut Mock,
        width: u32,
        progress: &PrintProgress,
        ms: u64,
    ) -> Result<Value> {
        execute_job(
            mock,
            &test_raster(width),
            "job",
            "usb:test",
            141,
            progress,
            tokio::time::Instant::now() + Duration::from_millis(ms),
        )
        .await
    }
    #[tokio::test]
    async fn transport_order_chunking_and_completion() {
        let mut mock = Mock {
            statuses: VecDeque::from([
                status(0, 0),
                status(1, 0),
                status(0, 0),
                status(3, 0),
                status(2, 0),
                status(0, 0),
            ]),
            ..Default::default()
        };
        let progress = PrintProgress::default();
        progress.set_resolved_device_id("usb:7:3".into());
        let output = run_mock(&mut mock, 1025, &progress, 1000).await.unwrap();
        assert_eq!(output["state"], "completed");
        assert_eq!(output["resolved_device_id"], "usb:7:3");
        assert_eq!(progress.snapshot()["resolved_device_id"], "usb:7:3");
        assert_eq!(mock.writes.len(), 5);
        assert_eq!(mock.writes[0], header(1025, 141));
        assert_eq!(mock.writes[1].len(), 1024 * 40);
        assert_eq!(mock.writes[2].len(), 40);
        assert_eq!(mock.writes[3], [0x0c]);
        assert_eq!(mock.writes[4], command(b'@', &[]));
        assert_eq!(
            progress.snapshot()["observed_st_transitions"],
            serde_json::json!([0, 1, 0, 3, 2, 0])
        );
    }
    #[tokio::test]
    async fn waiting_only_and_prior_busy_timeout() {
        for sequence in [vec![1, 0, 0, 0], vec![0, 3, 4, 5, 0]] {
            let mut mock = Mock {
                statuses: sequence.into_iter().map(|s| status(s, 0)).collect(),
                ..Default::default()
            };
            let p = PrintProgress::default();
            let err = run_mock(&mut mock, 1, &p, 300).await.unwrap_err();
            assert_eq!(err.exit_code, 7);
            assert_eq!(err.context.unwrap()["phase"], "wait_completion");
            assert_eq!(mock.writes.len(), 4);
        }
    }
    #[tokio::test]
    async fn transport_failures_are_not_retried() {
        assert_eq!(check_transfer(20, 20, false).unwrap_err().exit_code, 7);
        assert_eq!(check_transfer(20, 19, true).unwrap_err().exit_code, 7);
        for at in 1..=4 {
            let mut mock = Mock {
                fail_send_at: Some(at),
                statuses: VecDeque::from([status(0, 0)]),
                ..Default::default()
            };
            let p = PrintProgress::default();
            let err = run_mock(&mut mock, 1, &p, 200).await.unwrap_err();
            assert_eq!(err.exit_code, 7);
            assert_eq!(mock.writes.len(), at);
            assert_eq!(
                err.context.unwrap()["phase"],
                ["header", "raster", "form_feed", "final_reset"][at - 1]
            );
        }
        for status in [
            Ok(vec![22]),
            Ok(vec![0; 22]),
            status(0, 4),
            Err(AppError::unknown("disconnected")),
        ] {
            let mut mock = Mock {
                statuses: VecDeque::from([status]),
                ..Default::default()
            };
            let p = PrintProgress::default();
            let err = run_mock(&mut mock, 1, &p, 200).await.unwrap_err();
            assert!([5, 7].contains(&err.exit_code));
            assert_eq!(mock.writes.len(), 2);
            assert_eq!(err.context.unwrap()["phase"], "raster");
        }
    }
    #[tokio::test]
    async fn expired_deadline_never_submits_and_pending_send_times_out() {
        let mut mock = Mock::default();
        let p = PrintProgress::default();
        let err = execute_job(
            &mut mock,
            &test_raster(1),
            "job",
            "usb:test",
            141,
            &p,
            tokio::time::Instant::now() - Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(err.exit_code, 4);
        assert!(mock.writes.is_empty());
        assert_eq!(p.snapshot()["transmission_started"], false);

        let mut mock = Mock {
            pending_send_at: Some(1),
            ..Default::default()
        };
        let p = PrintProgress::default();
        let err = run_mock(&mut mock, 1, &p, 10).await.unwrap_err();
        assert_eq!(err.exit_code, 7);
        assert_eq!(mock.writes.len(), 1);
        assert_eq!(err.context.unwrap()["phase"], "header");
    }
    #[test]
    fn preflight_status_classification() {
        let mut data = [0u8; 22];
        data[0] = 22;
        data[6] = 5;
        data[8] = 5;
        data[11] = 1;
        assert!(validate_preflight(&data, 5).is_ok());
        assert_eq!(
            validate_preflight(&data[..21], 5).unwrap_err().code,
            "unsupported"
        );
        for st in 1..=5 {
            data[1] = st;
            assert_eq!(
                validate_preflight(&data, 5).unwrap_err().code,
                "printer_busy"
            );
        }
        data[1] = 9;
        assert_eq!(
            validate_preflight(&data, 5).unwrap_err().code,
            "unsupported"
        );
        data[1] = 0;
        data[2] = 1;
        assert_eq!(
            validate_preflight(&data, 5).unwrap_err().code,
            "device_error"
        );
        data[2] = 0;
        data[6] = 4;
        assert_eq!(
            validate_preflight(&data, 5).unwrap_err().code,
            "tape_mismatch"
        );
        data[6] = 5;
        data[11] = 0;
        assert_eq!(
            validate_preflight(&data, 5).unwrap_err().code,
            "unsupported"
        );
    }
    #[test]
    fn interruption_snapshots_are_shared() {
        let p = PrintProgress::default();
        let clone = p.clone();
        let before = p.interrupted();
        assert_eq!(before.exit_code, 4);
        assert_eq!(before.code, "print_interrupted");
        assert_eq!(before.context.unwrap()["state"], "not_submitted");
        clone.phase("raster");
        clone.started();
        clone.record(&[22, 3]);
        let after = p.interrupted();
        assert_eq!(after.exit_code, 7);
        let c = after.context.unwrap();
        assert_eq!(c["phase"], "raster");
        assert_eq!(c["transmission_started"], true);
        assert_eq!(c["raw_status"], serde_json::json!([22, 3]));
    }
    fn png(width: u32, pixel: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, width, HEAD);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_pixel_dims(Some(png::PixelDimensions {
                xppu: 14173,
                yppu: 14173,
                unit: png::Unit::Meter,
            }));
            let mut writer = enc.write_header().unwrap();
            writer
                .write_image_data(&vec![pixel; (width * HEAD) as usize])
                .unwrap();
        }
        bytes
    }
    #[test]
    fn validates_width_and_binary_pixels() {
        assert_eq!(decode(&png(MAX_LENGTH, 255)).unwrap().width, MAX_LENGTH);
        assert!(decode(&png(MAX_LENGTH + 1, 255)).is_err());
        assert!(decode(&png(1, 128)).is_err());
    }
    #[test]
    fn protocol_commands_and_transpose() {
        assert_eq!(
            command(b'L', &709u32.to_le_bytes()),
            [0x1b, 0x7b, 7, b'L', 0xc5, 2, 0, 0, 0x13, 0x7d]
        );
        assert_eq!(
            command(b'T', &0u16.to_le_bytes()),
            [0x1b, 0x7b, 5, b'T', 0, 0, b'T', 0x7d]
        );
        let mut pixels = vec![255; 512];
        pixels[0] = 0;
        pixels[1 + 7 * 2] = 0;
        pixels[255 * 2] = 0;
        let bytes = rows(&Raster { width: 2, pixels }, 0, 2);
        assert_eq!(&bytes[..8], &[0x1b, 0x2e, 0, 0, 0, 1, 0, 1]);
        assert_eq!(bytes[8], 0x80);
        assert_eq!(bytes[39], 1);
        assert_eq!(bytes[48], 0);
        assert_eq!(bytes[79], 0x80);
        // Source row 7 is at head position 248 (byte 31, most significant bit),
        // not at head position 7. This is the asymmetric orientation anchor.
        assert_eq!(bytes[40..48], [0x1b, 0x2e, 0, 0, 0, 1, 0, 1]);
        assert_eq!(&header(1, 0)[..7], &[0x1b, 0x7b, 3, 0x40, 0x40, 0x7d, 0x1b]);
        assert_eq!(
            &header(1, 141)[16..26],
            &[0x1b, 0x7b, 7, b'C', 1, 1, 1, 1, 0x47, 0x7d]
        );
        assert!(header(1, 14).ends_with(&[0x1b, 0x7b, 5, b'T', 1, 0, 0x55, 0x7d]));
        assert!(header(709, 14).ends_with(&[0x1b, 0x7b, 5, b'T', 14, 0, 0x62, 0x7d]));
    }
    #[test]
    fn completion_requires_busy_and_reports_error() {
        let mut state = CompletionState::default();
        let mut status = [0u8; 22];
        status[0] = 22;
        assert!(!state.observe(&status).unwrap());
        status[1] = 1;
        assert!(!state.observe(&status).unwrap());
        status[1] = 0;
        assert!(state.observe(&status).unwrap());
        assert_eq!(state.transitions, [0, 1, 0]);
        status[2] = 1;
        assert_eq!(state.observe(&status).unwrap_err().exit_code, 5);
        assert_eq!(state.observe(&[]).unwrap_err().exit_code, 7);
    }
    #[test]
    fn errors_retain_raw_status_phase_and_transitions() {
        let mut diagnostics = Diagnostics::default();
        let mut data = [0u8; 22];
        data[0] = 22;
        diagnostics.record(&data);
        diagnostics.phase = "raster";
        data[1] = 9;
        diagnostics.record(&data);
        let error = diagnostics.attach(CompletionState::default().observe(&data).unwrap_err());
        assert_eq!(error.exit_code, 7);
        let context = error.context.unwrap();
        assert_eq!(context["phase"], "raster");
        assert_eq!(context["raw_status"][1], 9);
        assert_eq!(
            context["observed_st_transitions"],
            serde_json::json!([0, 9])
        );
        diagnostics.phase = "wait_completion";
        data[2] = 4;
        diagnostics.record(&data);
        let error = diagnostics.attach(CompletionState::default().observe(&data).unwrap_err());
        assert_eq!(error.exit_code, 5);
        assert_eq!(error.context.unwrap()["raw_status"][2], 4);
        diagnostics.record(&[1]);
        let error = diagnostics.attach(CompletionState::default().observe(&[1]).unwrap_err());
        assert_eq!(error.context.unwrap()["raw_status"], serde_json::json!([1]));
        diagnostics.phase = "final_reset";
        let error = diagnostics.attach(AppError::unknown("timeout"));
        assert_eq!(error.context.unwrap()["phase"], "final_reset");
    }
    #[test]
    fn waiting_states_do_not_count_as_busy() {
        let mut data = [0u8; 22];
        data[0] = 22;
        let mut state = CompletionState::default();
        for st in [3, 4, 5, 0] {
            data[1] = st;
            assert!(!state.observe(&data).unwrap());
        }
        let mut state = CompletionState::default();
        for st in [1, 3, 4, 5] {
            data[1] = st;
            assert!(!state.observe(&data).unwrap());
        }
        data[1] = 0;
        assert!(state.observe(&data).unwrap());
    }

    #[test]
    fn observed_hardware_completion_sequences() {
        let evidence: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/usb/hardware-2026-11-10.json"
        ))
        .unwrap();
        for job in evidence["prints"].as_array().unwrap() {
            let final_status: Vec<u8> =
                serde_json::from_value(job["final_raw_status"].clone()).unwrap();
            validate_preflight(&final_status, 5).unwrap();
            let mut state = CompletionState::default();
            let transitions = job["observed_st_transitions"].as_array().unwrap();
            // Only ST transitions and the final full record were captured.
            // Reconstruct input records for this state-machine regression;
            // these are not presented as captured intermediate USB packets.
            for (index, st) in transitions.iter().enumerate() {
                let mut data = final_status.clone();
                data[1] = st.as_u64().unwrap() as u8;
                assert_eq!(
                    state.observe(&data).unwrap(),
                    index + 1 == transitions.len()
                );
            }
        }
    }

    #[test]
    fn observed_hardware_errors_fail_preflight() {
        let evidence: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/usb/hardware-2026-11-10.json"
        ))
        .unwrap();
        for case in evidence["preflight_failures"].as_array().unwrap() {
            let data: Vec<u8> = serde_json::from_value(case["raw_status"].clone()).unwrap();
            let error = validate_preflight(&data, 5).unwrap_err();
            assert_eq!(
                error.exit_code,
                case["expected_exit_code"].as_i64().unwrap() as i32
            );
            assert_eq!(error.code, case["error_code"]);
            assert!(error.message.contains(case["condition"].as_str().unwrap()));
        }
    }
}
