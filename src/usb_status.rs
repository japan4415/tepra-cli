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
    Ok(json!({"raw_bytes":data,
        "raw_fields":{"ST":data[1] as i8,"ER":data[2] as i8,"ER2":data[2] as i8,
            "EI2":data[4] as i8,"EJ2":data[5] as i8,"TW":data[6] as i8,"TR":data[7] as i8,
            "TS":data[8] as i8,"T8S":data[9] as i8,"T8O":data[10] as i8,"T8T":data[11] as i8,
            "PP":u16::from_be_bytes([data[12],data[13]]),
            "RB":u32::from_be_bytes([data[14],data[15],data[16],data[17]]),
            "WA":data[18] as i8,"VER":data[19] as i8,"RES":u16::from_be_bytes([data[20],data[21]])},
        "ready":null,"tape_width_mm":if data[6]==5 {Some(24)} else {None},"printing_completed":null,
        "interpretation":"TW=5 was verified against 24 mm tape. Other tape codes and full readiness semantics are not yet verified."}))
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
        let info = devices
            .into_iter()
            .find(|d| format!("usb:{}:{}", d.bus_id(), d.device_address()) == device_id)
            .ok_or_else(|| {
                AppError::new(
                    "device_not_found",
                    "USB device no longer present; run devices again",
                    3,
                )
            })?;
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
            json!({"device_id":device_id,"model":"sr-mk2","transport":"usb", "state":"observed",
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
        assert!(decode_status(&raw[..21]).is_err());
        assert!(decode_status(&[0; 22]).is_err());
    }
}
