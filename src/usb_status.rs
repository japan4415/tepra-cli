//! SR-MK2 vendor status query, observed on USB on 2026-11-10.
//!
//! The request and raw field names were located in TEPRA LINK 2 1.4.0.
//! This is a read request, not an implementation of the printing protocol.
use crate::error::{AppError, Result};
use serde_json::Value;
#[cfg(any(target_os = "macos", target_os = "linux", test))]
use serde_json::json;

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn decode_status(data: &[u8]) -> Result<Value> {
    if data.len() != 22 || data[0] != 22 {
        return Err(AppError::unsupported(
            "Unexpected SR-MK2 status response; expected a 22-byte status record",
        ));
    }
    // These labels describe the observed protocol role, not a complete
    // interpretation of every sensor or cartridge type.
    let activity = match data[1] {
        0 => "idle",
        1 | 2 => "busy",
        3..=5 => "waiting",
        _ => "unknown",
    };
    let observed_profile = data[6..12] == [5, 0, 5, 0, 0, 1];
    Ok(json!({"raw_bytes":data,
        "raw_fields":{"ST":data[1] as i8,"ER":data[2] as i8,"ER2":data[2] as i8,
            "EI2":data[4] as i8,"EJ2":data[5] as i8,"TW":data[6] as i8,"TR":data[7] as i8,
            "TS":data[8] as i8,"T8S":data[9] as i8,"T8O":data[10] as i8,"T8T":data[11] as i8,
            "PP":u16::from_be_bytes([data[12],data[13]]),
            "RB":u32::from_be_bytes([data[14],data[15],data[16],data[17]]),
            "WA":data[18] as i8,"VER":data[19] as i8,"RES":u16::from_be_bytes([data[20],data[21]])},
        "ready":null,"tape_width_mm":if data[6]==5 {Some(24)} else {None},"printing_completed":null,
        "summary":{
            "activity":activity,
            "error":if data[2]==0 {"none_reported"} else {"unknown_device_error"},
            "error_code":data[2],
            "cartridge_profile":if observed_profile {"observed_24mm"} else {"unknown"},
            "experimental_usb_preflight_matches":data[1]==0 && data[2]==0 && observed_profile
        },
        "interpretation":"ST=0 is idle, 1/2 busy, 3/4/5 waiting. TW=5 was observed with 24 mm tape. Nonzero ER reports an error whose physical cause is unknown. Profile matching is not a readiness or completion guarantee."}))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub async fn status(device_id: &str) -> Result<Value> {
    use nusb::transfer::{ControlIn, ControlType, Recipient};
    use std::{future::IntoFuture, time::Duration};
    if !device_id.starts_with("usb:") {
        return Err(AppError::unsupported(
            "Direct status is currently implemented for SR-MK2 USB on macOS/Linux only",
        ));
    }
    let operation = async {
        let devices = nusb::list_devices().await.map_err(connection_error)?;
        let devices: Vec<_> = devices.collect();
        let info = crate::discovery::resolve_usb_device(device_id, &devices)?;
        let resolved_device_id = format!("usb:{}:{}", info.bus_id(), info.device_address());
        if info.vendor_id() != 0x0d8a || info.product_id() != 0x0103 {
            return Err(AppError::new(
                "printer_mismatch",
                "Status query is restricted to SR-MK2 USB 0d8a:0103",
                3,
            ));
        }
        let device = info.open().await.map_err(connection_error)?;
        let config = device.active_configuration().map_err(connection_error)?;
        let matches_profile = config.interface_alt_settings().any(|i| {
            i.interface_number() == 0
                && i.alternate_setting() == 0
                && i.class() == 7
                && i.subclass() == 1
                && i.protocol() == 2
        });
        if !matches_profile {
            return Err(AppError::unsupported(
                "Unexpected SR-MK2 USB interface configuration",
            ));
        }
        let bytes = device
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
            .map_err(connection_error)?;
        Ok(
            json!({"device_id":device_id,"resolved_device_id":resolved_device_id,"model":"sr-mk2","transport":"usb", "state":"observed",
            "status":decode_status(&bytes)?,"direct_print_supported":true,"requires_experimental_usb_flag":true}),
        )
    };
    tokio::time::timeout(Duration::from_secs(10), operation)
        .await
        .map_err(|_| AppError::new("device_timeout", "USB status query timed out", 4))?
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub async fn status(_device_id: &str) -> Result<Value> {
    Err(AppError::unsupported(
        "Direct status is currently implemented for SR-MK2 USB on macOS/Linux only",
    ))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn connection_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(
        "device_unavailable",
        format!("USB status could not be read: {error}"),
        4,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_observed_record_without_inventing_readiness() {
        let raw = [
            22, 0, 0, 0, 0, 0, 5, 0, 5, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0,
        ];
        let status = decode_status(&raw).unwrap();
        assert_eq!(status["raw_fields"]["ST"], 0);
        assert_eq!(status["raw_fields"]["TW"], 5);
        assert!(status["ready"].is_null());
        assert_eq!(status["summary"]["activity"], "idle");
        assert_eq!(status["summary"]["cartridge_profile"], "observed_24mm");
        assert_eq!(
            status["summary"]["experimental_usb_preflight_matches"],
            true
        );
        assert!(decode_status(&raw[..21]).is_err());
        assert!(decode_status(&[0; 22]).is_err());
    }

    #[test]
    fn unknown_status_fields_are_not_guessed() {
        let mut raw = [0; 22];
        raw[0] = 22;
        raw[1] = 200;
        raw[2] = 201;
        raw[6] = 202;
        let status = decode_status(&raw).unwrap();
        assert_eq!(status["summary"]["activity"], "unknown");
        assert_eq!(status["summary"]["error"], "unknown_device_error");
        assert_eq!(status["summary"]["error_code"], 201);
        assert_eq!(status["summary"]["cartridge_profile"], "unknown");
        assert!(status["tape_width_mm"].is_null());
        assert!(status["ready"].is_null());
        assert!(status["printing_completed"].is_null());
        assert_eq!(status["raw_bytes"][1], 200);
        assert_eq!(status["raw_fields"]["ST"], -56);
        // A known width alone must not make an unknown cartridge eligible.
        raw[1] = 0;
        raw[2] = 0;
        raw[6] = 5;
        let status = decode_status(&raw).unwrap();
        assert_eq!(status["tape_width_mm"], 24);
        assert_eq!(
            status["summary"]["experimental_usb_preflight_matches"],
            false
        );
        for (st, expected) in [
            (1, "busy"),
            (2, "busy"),
            (3, "waiting"),
            (4, "waiting"),
            (5, "waiting"),
        ] {
            raw[1] = st;
            let status = decode_status(&raw).unwrap();
            assert_eq!(status["summary"]["activity"], expected);
            assert_eq!(
                status["summary"]["experimental_usb_preflight_matches"],
                false
            );
        }
    }
}
