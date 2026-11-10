//! Experimental SR-MK2 USB protocol. Restricted to the observed 24 mm profile.
use crate::error::{AppError, Result};
use serde_json::Value;
use std::time::Duration;

const MAX_LENGTH: u32 = 1417;
const HEAD: u32 = 256;
const EXPECTED_TAPE_CODE: Option<u8> = Some(5);
// TEPRA LINK 2 default: truncate(360 * 10 / 25.4), in dots.
// Physical leading/trailing feed is not implied by this parameter alone.
const PAGE_MARGIN: Option<u16> = Some(141);

/// Offline validation for the experimental USB profile; never opens a device.
pub fn validate(png: &[u8]) -> Result<()> {
    decode(png).map(|_| ())
}

struct Diagnostics {
    phase: &'static str,
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
        error.with_context(serde_json::json!({"phase":self.phase,"raw_status":self.raw_status,"observed_st_transitions":self.transitions,"observed_st_transitions_truncated":self.transitions_truncated}))
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
                if r.pixels[((y + bit) * r.width + x) as usize] == 0 {
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
                    "Printer reported error {}; physical result may be partial",
                    data[2]
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

pub async fn print(device_id: &str, png: &[u8], job_id: &str, timeout: Duration) -> Result<Value> {
    let raster = decode(png).map_err(|e| Diagnostics::default().attach(e))?;
    let (Some(tape), Some(margin)) = (EXPECTED_TAPE_CODE, PAGE_MARGIN) else {
        return Err(Diagnostics::default().attach(AppError::unsupported(
            "Experimental USB printing is disabled until the 24 mm tape code and page margin are verified",
        )));
    };
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        usb_print(device_id, &raster, job_id, timeout, tape, margin).await
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (device_id, raster, job_id, timeout, tape, margin);
        Err(Diagnostics::default().attach(AppError::unsupported(
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
) -> Result<Value> {
    use nusb::{
        descriptors::TransferType,
        transfer::{Bulk, ControlIn, ControlType, Out, Recipient},
    };
    use std::future::IntoFuture;
    let deadline = tokio::time::Instant::now() + timeout;
    let mut diagnostics = Diagnostics::default();
    let unavailable = |e: String| AppError::new("device_unavailable", e, 4);
    let preflight = async {
        let devices = nusb::list_devices()
            .await
            .map_err(|e| unavailable(e.to_string()))?;
        let info = devices
            .into_iter()
            .find(|d| format!("usb:{}:{}", d.bus_id(), d.device_address()) == device_id)
            .ok_or_else(|| AppError::new("device_not_found", "USB device no longer present", 3))?;
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
        diagnostics.record(&data);
        if data.len() != 22 || data[0] != 22 {
            return Err(AppError::unsupported("Unexpected SR-MK2 status"));
        }
        if data[2] != 0 {
            return Err(AppError::new(
                "device_error",
                format!("Printer reported error {}", data[2]),
                5,
            ));
        }
        if data[1] != 0 {
            return Err(AppError::new("printer_busy", "Printer is not idle", 4));
        }
        if data[6] != tape {
            return Err(AppError::new("tape_mismatch", "Expected 24 mm tape", 5));
        }
        if data[7] != 0 || data[8] != 5 || data[9] != 0 || data[10] != 0 || data[11] != 1 {
            return Err(AppError::unsupported(
                "Printer is busy or tape is not the observed 24 mm cartridge profile",
            ));
        }
        let interface = device
            .claim_interface(0)
            .await
            .map_err(|e| unavailable(e.to_string()))?;
        Ok((device, interface))
    };
    let preflight_result = tokio::time::timeout_at(deadline, preflight).await;
    let (device, interface) = preflight_result
        .map_err(|_| {
            diagnostics.attach(AppError::new(
                "device_timeout",
                "USB preflight timed out",
                4,
            ))
        })?
        .map_err(|e| diagnostics.attach(e))?;
    let mut ep = interface
        .endpoint::<Bulk, Out>(0x02)
        .map_err(|e| diagnostics.attach(unavailable(e.to_string())))?;
    let operation = async {
        async fn send(ep: &mut nusb::Endpoint<Bulk, Out>, data: Vec<u8>) -> Result<()> {
            let len = data.len();
            ep.submit(data.into());
            let completion = ep.next_complete().await;
            if completion.status.is_err() || completion.actual_len != len {
                return Err(AppError::unknown(
                    "USB transfer failed or was partial; physical result may be partial",
                ));
            }
            Ok(())
        }
        async fn status(device: &nusb::Device) -> Result<Vec<u8>> {
            device
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
        diagnostics.phase = "header";
        send(&mut ep, header(raster.width, margin)).await?;
        for start in (0..raster.width).step_by(1024) {
            diagnostics.phase = "raster";
            send(
                &mut ep,
                rows(raster, start, (start + 1024).min(raster.width)),
            )
            .await?;
            let data = status(&device).await?;
            diagnostics.record(&data);
            CompletionState::default().observe(&data)?;
        }
        diagnostics.phase = "form_feed";
        send(&mut ep, vec![0x0c]).await?;
        diagnostics.phase = "final_reset";
        send(&mut ep, command(b'@', &[])).await?;
        diagnostics.phase = "wait_completion";
        let mut state = CompletionState::default();
        let final_status = loop {
            let data = status(&device).await?;
            diagnostics.record(&data);
            if state.observe(&data)? {
                break data;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        Ok(
            serde_json::json!({"job_id":job_id, "device_id":device_id, "state":"completed", "completion_evidence":"usb_busy_then_idle", "final_status":{"raw_bytes": final_status}, "observed_st_transitions":state.transitions, "observed_st_transitions_truncated":state.transitions_truncated}),
        )
    };
    let outcome = tokio::time::timeout_at(deadline, operation).await;
    outcome
        .map_err(|_| {
            diagnostics.attach(AppError::unknown(
                "USB print timed out after transmission; physical result may be partial",
            ))
        })?
        .map_err(|e| diagnostics.attach(e))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(bytes[48], 1);
        assert_eq!(&header(1, 0)[..7], &[0x1b, 0x7b, 3, 0x40, 0x40, 0x7d, 0x1b]);
        assert_eq!(
            &header(1, 141)[16..26],
            &[0x1b, 0x7b, 7, b'C', 1, 1, 1, 1, 0x47, 0x7d]
        );
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
}
