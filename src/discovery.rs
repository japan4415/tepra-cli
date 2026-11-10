//! Read-only USB and Bluetooth discovery. No printer commands are sent.
use crate::error::{AppError, Result};
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use serde_json::{Value, json};
use std::{
    future::{Future, IntoFuture},
    time::Duration,
};

const DEADLINE: Duration = Duration::from_secs(10);
const MAX_SCAN_SECONDS: u64 = 30;

fn unavailable(message: impl Into<String>) -> AppError {
    AppError::new("device_unavailable", message, 4)
}
fn not_found(id: &str) -> AppError {
    AppError::new("device_not_found", format!("Device not found: {id}"), 3)
}
async fn bounded<T, E: std::fmt::Display>(
    operation: &str,
    future: impl Future<Output = std::result::Result<T, E>>,
) -> Result<T> {
    tokio::time::timeout(DEADLINE, future)
        .await
        .map_err(|_| unavailable(format!("{operation} timed out")))?
        .map_err(|e| unavailable(format!("{operation}: {e}")))
}
fn scan_duration(seconds: u64) -> Result<Duration> {
    if !(1..=MAX_SCAN_SECONDS).contains(&seconds) {
        return Err(AppError::invalid(format!(
            "scan_seconds must be between 1 and {MAX_SCAN_SECONDS}"
        )));
    }
    Ok(Duration::from_secs(seconds))
}
fn usb_id(bus: &str, address: u8) -> String {
    format!("usb:{bus}:{address}")
}
fn ble_id(adapter: usize, id: &str) -> String {
    format!("ble:{adapter}:{id}")
}
// A name is only a hint, never a verified model identification.
fn candidate(name: Option<&str>) -> bool {
    name.is_some_and(|n| {
        let n = n.to_ascii_lowercase();
        n.contains("tepra") || n.contains("sr-mk2") || n.contains("king jim")
    })
}
fn usb_info(info: &nusb::DeviceInfo) -> Value {
    // Observed on a real SR-MK2, 2026-11-10. This identifies the model,
    // not the correctness of any unverified print protocol.
    let known_mk2 = info.vendor_id() == 0x0d8a && info.product_id() == 0x0103;
    let hint = candidate(info.product_string()) || candidate(info.manufacturer_string());
    json!({"id": usb_id(info.bus_id(), info.device_address()), "transport":"usb", "bus_id":info.bus_id(), "device_address":info.device_address(), "vendor_id":info.vendor_id(), "product_id":info.product_id(), "class":info.class(), "subclass":info.subclass(), "protocol":info.protocol(), "manufacturer":info.manufacturer_string(), "product":info.product_string(), "serial_number":info.serial_number(), "candidate":known_mk2 || hint, "name_hint":hint, "model":if known_mk2 {"sr-mk2"} else {"unknown"}, "identification":if known_mk2 {"observed-usb-vid-pid"} else {"unverified"}, "direct_print_supported":known_mk2 && cfg!(any(target_os="macos", target_os="linux")), "requires_experimental_usb_flag":true})
}
async fn usb_devices() -> Result<Vec<nusb::DeviceInfo>> {
    Ok(
        bounded("USB enumeration", nusb::list_devices().into_future())
            .await?
            .collect(),
    )
}
async fn usb_doctor(id: &str) -> Result<Value> {
    let info = usb_devices()
        .await?
        .into_iter()
        .find(|d| usb_id(d.bus_id(), d.device_address()) == id)
        .ok_or_else(|| not_found(id))?;
    let device = bounded("USB open", info.open().into_future()).await?;
    let configurations: Vec<Value> = device.configurations().map(|c| json!({
        "configuration_value":c.configuration_value(),
        "interfaces": c.interface_alt_settings().map(|i| json!({"interface_number":i.interface_number(), "alternate_setting":i.alternate_setting(), "class":i.class(), "subclass":i.subclass(), "protocol":i.protocol(), "endpoints":i.endpoints().map(|e| json!({"address":e.address(), "transfer_type":format!("{:?}", e.transfer_type()), "max_packet_size":e.max_packet_size(), "interval":e.interval()})).collect::<Vec<_>>() })).collect::<Vec<_>>()
    })).collect();
    Ok(
        json!({"device":usb_info(&info), "configurations":configurations, "active_configuration":device.active_configuration().ok().map(|c|c.configuration_value())}),
    )
}
async fn ble_scan(seconds: u64) -> Result<Vec<(usize, Peripheral)>> {
    let duration = scan_duration(seconds)?;
    let manager = bounded("Bluetooth manager", Manager::new()).await?;
    let adapters = bounded("Bluetooth adapters", manager.adapters()).await?;
    if adapters.is_empty() {
        return Err(unavailable("No Bluetooth adapter available"));
    }
    let mut started: Vec<(usize, Adapter)> = Vec::new();
    let mut failure = None;
    for (index, adapter) in adapters.into_iter().enumerate() {
        // Attempt stop even if starting times out: the OS may have started asynchronously.
        let result = bounded(
            "Bluetooth start scan",
            adapter.start_scan(ScanFilter::default()),
        )
        .await;
        started.push((index, adapter));
        if let Err(e) = result {
            failure = Some(e);
            break;
        }
    }
    if failure.is_none() {
        tokio::time::sleep(duration).await;
    }
    let mut found = Vec::new();
    if failure.is_none() {
        for (index, adapter) in &started {
            match bounded("Bluetooth peripherals", adapter.peripherals()).await {
                Ok(peripherals) => found.extend(peripherals.into_iter().map(|p| (*index, p))),
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
    }
    for (_, adapter) in &started {
        if let Err(e) = bounded("Bluetooth stop scan", adapter.stop_scan()).await
            && failure.is_none()
        {
            failure = Some(e);
        }
    }
    if let Some(e) = failure {
        return Err(e);
    }
    Ok(found)
}
async fn ble_info(adapter: usize, p: &Peripheral) -> Result<Value> {
    let props = bounded("Bluetooth advertising properties", p.properties()).await?;
    let advertising = props.map(|v| json!({"local_name":v.local_name, "advertisement_name":v.advertisement_name, "appearance":v.appearance, "tx_power_level":v.tx_power_level, "rssi":v.rssi, "manufacturer_data":v.manufacturer_data, "service_data":v.service_data.into_iter().map(|(k,v)| (k.to_string(),v)).collect::<std::collections::BTreeMap<_,_>>(), "services":v.services.iter().map(ToString::to_string).collect::<Vec<_>>(), "class":v.class}));
    let hint = advertising.as_ref().is_some_and(|a| {
        candidate(a["local_name"].as_str()) || candidate(a["advertisement_name"].as_str())
    });
    Ok(
        json!({"id":ble_id(adapter, &p.id().to_string()), "transport":"ble", "advertising":advertising, "candidate":hint, "name_hint":hint, "model":"unknown", "identification":"unverified", "direct_print_supported":false}),
    )
}
async fn ble_doctor(id: &str, seconds: u64) -> Result<Value> {
    let (adapter, p) = ble_scan(seconds)
        .await?
        .into_iter()
        .find(|(a, p)| ble_id(*a, &p.id().to_string()) == id)
        .ok_or_else(|| not_found(id))?;
    let info = ble_info(adapter, &p).await?;
    let result = async {
        bounded("Bluetooth connect", p.connect()).await?;
        bounded("Bluetooth service discovery", p.discover_services()).await?;
        let services: Vec<_> = p.services().iter().map(|s| json!({"uuid":s.uuid.to_string(), "primary":s.primary, "characteristics":s.characteristics.iter().map(|c| json!({"uuid":c.uuid.to_string(), "properties":format!("{:?}", c.properties), "descriptors":c.descriptors.iter().map(|d|d.uuid.to_string()).collect::<Vec<_>>() })).collect::<Vec<_>>() })).collect();
        Ok(json!({"device":info, "services":services}))
    }.await;
    // Also try after connect failure or timeout, which may have established a connection.
    let cleanup = bounded("Bluetooth disconnect", p.disconnect()).await;
    match result {
        Err(e) => Err(e),
        Ok(value) => {
            cleanup?;
            Ok(value)
        }
    }
}
pub async fn devices(transport: &str, scan_seconds: u64) -> Result<Value> {
    match transport {
        "usb" => {
            Ok(json!({"devices":usb_devices().await?.iter().map(usb_info).collect::<Vec<_>>() }))
        }
        "ble" => {
            let mut output = Vec::new();
            for (a, p) in ble_scan(scan_seconds).await? {
                output.push(ble_info(a, &p).await?);
            }
            Ok(json!({"devices":output}))
        }
        _ => Err(AppError::unsupported(format!(
            "Unsupported transport: {transport}"
        ))),
    }
}
pub async fn doctor(device_id: &str, scan_seconds: u64) -> Result<Value> {
    if device_id.starts_with("usb:") {
        usb_doctor(device_id).await
    } else if device_id.starts_with("ble:") {
        ble_doctor(device_id, scan_seconds).await
    } else {
        Err(AppError::invalid("Device ID must start with usb: or ble:"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_preserve_os_identity() {
        assert_eq!(usb_id("bus:port", 7), "usb:bus:port:7");
        assert_eq!(ble_id(1, "A:B"), "ble:1:A:B");
        assert_ne!(ble_id(0, "A:B"), ble_id(1, "A:B"));
    }
    #[test]
    fn names_are_only_hints() {
        assert!(candidate(Some("TEPRA SR-MK2")));
        assert!(!candidate(Some("other")));
        assert!(!candidate(None));
    }
    #[test]
    fn scan_is_bounded() {
        assert!(scan_duration(0).is_err());
        assert!(scan_duration(31).is_err());
        assert!(scan_duration(1).is_ok());
        assert!(scan_duration(30).is_ok());
    }
}
