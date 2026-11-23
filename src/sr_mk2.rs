//! Shared SR-MK2 constants, binary raster decoding, and USB device setup.
use crate::{
    error::{AppError, Result},
    render::{HEAD_DOTS, MAX_FILE_BYTES},
};

pub const VENDOR_ID: u16 = 0x0d8a;
pub const PRODUCT_ID: u16 = 0x0103;
pub const PNG_PPM_360DPI: u32 = 14173;

#[cfg_attr(
    not(any(target_os = "macos", target_os = "linux", test)),
    allow(dead_code)
)]
pub const STATUS_LEN: usize = 22;
#[cfg_attr(
    not(any(target_os = "macos", target_os = "linux", test)),
    allow(dead_code)
)]
pub const OBSERVED_24MM_TAPE_CODE: u8 = 5;
#[cfg_attr(
    not(any(target_os = "macos", target_os = "linux", test)),
    allow(dead_code)
)]
pub const OBSERVED_24MM_CARTRIDGE_PROFILE: [u8; 6] = [5, 0, 5, 0, 0, 1];
// TEPRA LINK 2 image-data API default: truncate(360 * 1 / 25.4), in dots.
// Physical leading/trailing feed is not implied by this parameter alone.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub const PAGE_MARGIN_DOTS: u16 = 14;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub const INTERFACE_NUMBER: u8 = 0;
#[cfg(any(target_os = "macos", target_os = "linux"))]
const INTERFACE_CLASS_PRINTER: u8 = 7;
#[cfg(any(target_os = "macos", target_os = "linux"))]
const INTERFACE_SUBCLASS_PRINTER: u8 = 1;
#[cfg(any(target_os = "macos", target_os = "linux"))]
const INTERFACE_PROTOCOL_BIDIR: u8 = 2;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub const BULK_IN_ENDPOINT: u8 = 0x81;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub const BULK_OUT_ENDPOINT: u8 = 0x02;
#[cfg(any(target_os = "macos", target_os = "linux"))]
const MAX_PACKET_SIZE: usize = 64;

#[derive(Debug)]
pub struct DecodedRaster {
    #[cfg_attr(
        not(any(target_os = "macos", target_os = "linux", test)),
        allow(dead_code)
    )]
    pub width: u32,
    pub pixels: Vec<u8>,
}

/// Decode and validate a 360 dpi, 256-dot-high 8-bit grayscale binary PNG.
pub fn decode_binary_raster(
    png: &[u8],
    expected_width: Option<u32>,
    max_width: Option<u32>,
) -> Result<DecodedRaster> {
    if png.len() > MAX_FILE_BYTES as usize {
        return Err(AppError::invalid("raster PNG exceeds size limit"));
    }
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder
        .read_info()
        .map_err(|e| AppError::invalid(format!("invalid raster PNG: {e}")))?;
    let info = reader.info();
    let width = info.width;
    let width_ok = width > 0
        && expected_width.is_none_or(|expected| width == expected)
        && max_width.is_none_or(|max| width <= max);
    if !width_ok
        || info.height != HEAD_DOTS
        || info.color_type != png::ColorType::Grayscale
        || info.bit_depth != png::BitDepth::Eight
        || info.trns.is_some()
        || info.animation_control.is_some()
        || info.frame_control.is_some()
        || !matches!(
            info.pixel_dims,
            Some(png::PixelDimensions {
                xppu: PNG_PPM_360DPI,
                yppu: PNG_PPM_360DPI,
                unit: png::Unit::Meter
            })
        )
    {
        return Err(AppError::invalid(
            "raster PNG format, dimensions or resolution mismatch",
        ));
    }
    let expected_size = width
        .checked_mul(HEAD_DOTS)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| AppError::invalid("invalid raster buffer size"))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| AppError::invalid("invalid raster buffer size"))?;
    if size != expected_size {
        return Err(AppError::invalid("invalid raster buffer size"));
    }
    let mut pixels = vec![0; size];
    reader
        .next_frame(&mut pixels)
        .map_err(|e| AppError::invalid(format!("invalid raster PNG: {e}")))?;
    if pixels.iter().any(|&v| v != 0 && v != 255) {
        return Err(AppError::invalid("raster is not binary"));
    }
    Ok(DecodedRaster { width, pixels })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn status_control_in() -> nusb::transfer::ControlIn {
    use nusb::transfer::{ControlIn, ControlType, Recipient};
    ControlIn {
        control_type: ControlType::Vendor,
        recipient: Recipient::Interface,
        request: 1,
        value: 0,
        index: u16::from(INTERFACE_NUMBER),
        length: STATUS_LEN as u16,
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub async fn open_usb(
    device_id: &str,
    require_bulk_endpoints: bool,
    mismatch_message: &str,
    unavailable: impl Fn(String) -> AppError,
) -> Result<(String, nusb::Device)> {
    use nusb::descriptors::TransferType;
    let devices: Vec<_> = nusb::list_devices()
        .await
        .map_err(|e| unavailable(e.to_string()))?
        .collect();
    let info = crate::discovery::resolve_usb_device(device_id, &devices)?;
    let resolved_device_id = format!("usb:{}:{}", info.bus_id(), info.device_address());
    if info.vendor_id() != VENDOR_ID || info.product_id() != PRODUCT_ID {
        return Err(AppError::new("printer_mismatch", mismatch_message, 3));
    }
    let device = info.open().await.map_err(|e| unavailable(e.to_string()))?;
    let config = device
        .active_configuration()
        .map_err(|e| unavailable(e.to_string()))?;
    let interface_matches = config.configuration_value() == 1
        && config.interface_alt_settings().any(|i| {
            i.interface_number() == INTERFACE_NUMBER
                && i.alternate_setting() == 0
                && i.class() == INTERFACE_CLASS_PRINTER
                && i.subclass() == INTERFACE_SUBCLASS_PRINTER
                && i.protocol() == INTERFACE_PROTOCOL_BIDIR
                && (!require_bulk_endpoints
                    || [BULK_IN_ENDPOINT, BULK_OUT_ENDPOINT].iter().all(|addr| {
                        i.endpoints().any(|e| {
                            e.address() == *addr
                                && e.transfer_type() == TransferType::Bulk
                                && e.max_packet_size() == MAX_PACKET_SIZE
                        })
                    }))
        });
    if !interface_matches {
        return Err(AppError::unsupported(
            "Unexpected SR-MK2 USB interface configuration",
        ));
    }
    Ok((resolved_device_id, device))
}
