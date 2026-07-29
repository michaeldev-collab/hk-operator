//! BlueZ helpers for Cyberdeck Pad hybrid GATT (same link as HID keyboard).

use bluer::{Adapter, Address, Device, Session};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;
use uuid::Uuid;

pub const DEVICE_NAME: &str = "Cyberdeck Pad";

pub const SERVICE_UUID: &str = "c0de0001-3d17-4a00-8000-00805f9b34fb";
pub const SLOTS_UUID: &str = "c0de0002-3d17-4a00-8000-00805f9b34fb";
pub const MACRO_EVENT_UUID: &str = "c0de0003-3d17-4a00-8000-00805f9b34fb";
pub const INFO_UUID: &str = "c0de0004-3d17-4a00-8000-00805f9b34fb";

pub const MODE_HID: u8 = 0;
pub const MODE_MACRO: u8 = 1;
pub const SLOT_BYTES: usize = 27;
pub const PRESET_COUNT: usize = 6;
pub const ACTION_COUNT: usize = 3;
pub const SLOT_COUNT: usize = PRESET_COUNT * ACTION_COUNT; // 18
pub const SLOTS_BYTES: usize = SLOT_BYTES * SLOT_COUNT; // 486

#[derive(Debug, Error)]
pub enum BleError {
    #[error("bluetooth: {0}")]
    Bluer(#[from] bluer::Error),
    #[error("device not found (look for bonded '{DEVICE_NAME}')")]
    DeviceNotFound,
    #[error("GATT service {0} not found — flash hybrid firmware?")]
    ServiceMissing(String),
    #[error("characteristic {0} missing")]
    CharMissing(String),
    #[error("slots payload length {got}, expected {SLOTS_BYTES}")]
    BadSlotsLen { got: usize },
    #[error("invalid slot index")]
    BadIndex,
    #[error("{0}")]
    Msg(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HotkeySlot {
    pub mode: u8,
    pub r#mod: u8,
    pub key: u8,
    pub label: String,
}

impl HotkeySlot {
    pub fn pack(&self) -> [u8; SLOT_BYTES] {
        let mut buf = [0u8; SLOT_BYTES];
        buf[0] = if self.mode == MODE_MACRO {
            MODE_MACRO
        } else {
            MODE_HID
        };
        buf[1] = self.r#mod;
        buf[2] = self.key;
        let bytes = self.label.as_bytes();
        let n = bytes.len().min(23);
        buf[3..3 + n].copy_from_slice(&bytes[..n]);
        buf
    }

    pub fn unpack(buf: &[u8]) -> Result<Self, BleError> {
        if buf.len() < SLOT_BYTES {
            return Err(BleError::BadSlotsLen { got: buf.len() });
        }
        let label_raw = &buf[3..SLOT_BYTES];
        let end = label_raw.iter().position(|&b| b == 0).unwrap_or(label_raw.len());
        let label = String::from_utf8_lossy(&label_raw[..end]).into_owned();
        Ok(Self {
            mode: buf[0],
            r#mod: buf[1],
            key: buf[2],
            label,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PadSlots {
    pub slots: Vec<HotkeySlot>, // len 18, index = p*3+a
}

impl PadSlots {
    pub fn get(&self, preset: usize, action: usize) -> Result<&HotkeySlot, BleError> {
        if preset >= PRESET_COUNT || action >= ACTION_COUNT {
            return Err(BleError::BadIndex);
        }
        self.slots
            .get(preset * ACTION_COUNT + action)
            .ok_or(BleError::BadIndex)
    }

    pub fn get_mut(&mut self, preset: usize, action: usize) -> Result<&mut HotkeySlot, BleError> {
        if preset >= PRESET_COUNT || action >= ACTION_COUNT {
            return Err(BleError::BadIndex);
        }
        self.slots
            .get_mut(preset * ACTION_COUNT + action)
            .ok_or(BleError::BadIndex)
    }

    pub fn pack(&self) -> Result<[u8; SLOTS_BYTES], BleError> {
        if self.slots.len() != SLOT_COUNT {
            return Err(BleError::BadSlotsLen {
                got: self.slots.len() * SLOT_BYTES,
            });
        }
        let mut out = [0u8; SLOTS_BYTES];
        for (i, slot) in self.slots.iter().enumerate() {
            let packed = slot.pack();
            let start = i * SLOT_BYTES;
            out[start..start + SLOT_BYTES].copy_from_slice(&packed);
        }
        Ok(out)
    }

    pub fn unpack(buf: &[u8]) -> Result<Self, BleError> {
        if buf.len() < SLOTS_BYTES {
            return Err(BleError::BadSlotsLen { got: buf.len() });
        }
        let mut slots = Vec::with_capacity(SLOT_COUNT);
        for i in 0..SLOT_COUNT {
            let start = i * SLOT_BYTES;
            slots.push(HotkeySlot::unpack(&buf[start..start + SLOT_BYTES])?);
        }
        Ok(Self { slots })
    }

    pub fn binding_key(preset: usize, action: usize) -> String {
        format!("{preset}-{action}")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MacroEvent {
    pub preset: u8,
    pub action: u8,
}

impl MacroEvent {
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < 2 {
            return None;
        }
        Some(Self {
            preset: buf[0],
            action: buf[1],
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PadStatus {
    pub address: String,
    pub name: Option<String>,
    pub connected: bool,
    pub paired: bool,
    pub info: Option<String>,
}

/// Redact a Bluetooth device address for UI/logs (P3-08).
///
/// Keeps the last octet so multiple pads remain distinguishable without
/// exposing the full MAC. Non-MAC-shaped strings become fully masked.
pub fn redact_ble_address(addr: &str) -> String {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return "(no address)".into();
    }
    let parts: Vec<&str> = trimmed
        .split(|c| c == ':' || c == '-')
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
    {
        format!("**:**:**:**:**:{}", parts[5].to_ascii_uppercase())
    } else {
        "**:**:**:**:**:**".into()
    }
}

/// Clone status with a redacted address for probe/log output.
pub fn pad_status_for_log(st: &PadStatus) -> PadStatus {
    PadStatus {
        address: redact_ble_address(&st.address),
        name: st.name.clone(),
        connected: st.connected,
        paired: st.paired,
        info: st.info.clone(),
    }
}

pub struct CyberdeckPad {
    pub address: Address,
    device: Device,
}

impl CyberdeckPad {
    pub async fn session_adapter() -> Result<(Session, Adapter), BleError> {
        let session = Session::new().await?;
        let adapter = session.default_adapter().await?;
        adapter.set_powered(true).await?;
        Ok((session, adapter))
    }

    pub async fn find(adapter: &Adapter) -> Result<Self, BleError> {
        let addrs = adapter.device_addresses().await?;
        let mut fallback: Option<(Address, Device)> = None;
        for addr in addrs {
            let device = adapter.device(addr)?;
            let name = device.name().await?.unwrap_or_default();
            if name == DEVICE_NAME {
                return Ok(Self {
                    address: addr,
                    device,
                });
            }
            // Alias / stale name still usable if address known later
            if name.to_lowercase().contains("cyberdeck") {
                fallback = Some((addr, device));
            }
        }
        if let Some((address, device)) = fallback {
            return Ok(Self { address, device });
        }
        Err(BleError::DeviceNotFound)
    }

    pub async fn find_by_address(adapter: &Adapter, addr: &str) -> Result<Self, BleError> {
        let address = Address::from_str(addr)
            .map_err(|e| BleError::Msg(format!("bad address {addr}: {e}")))?;
        let device = adapter.device(address)?;
        Ok(Self { address, device })
    }

    pub async fn ensure_connected(&self) -> Result<(), BleError> {
        if !self.device.is_connected().await? {
            // Prefer using the existing HID bond — connect() reuses BlueZ session.
            self.device.connect().await?;
        }
        // Wait briefly for services to resolve
        for _ in 0..20 {
            if !self.device.services().await?.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok(())
    }

    pub async fn status(&self) -> Result<PadStatus, BleError> {
        let connected = self.device.is_connected().await?;
        let paired = self.device.is_paired().await?;
        let name = self.device.name().await?;
        let info = if connected {
            self.read_info().await.ok()
        } else {
            None
        };
        Ok(PadStatus {
            address: self.address.to_string(),
            name,
            connected,
            paired,
            info,
        })
    }

    async fn find_char(
        &self,
        service_uuid: Uuid,
        char_uuid: Uuid,
    ) -> Result<bluer::gatt::remote::Characteristic, BleError> {
        self.ensure_connected().await?;
        for service in self.device.services().await? {
            if service.uuid().await? != service_uuid {
                continue;
            }
            for ch in service.characteristics().await? {
                if ch.uuid().await? == char_uuid {
                    return Ok(ch);
                }
            }
            return Err(BleError::CharMissing(char_uuid.to_string()));
        }
        Err(BleError::ServiceMissing(service_uuid.to_string()))
    }

    pub async fn read_info(&self) -> Result<String, BleError> {
        let ch = self
            .find_char(Uuid::parse_str(SERVICE_UUID).unwrap(), Uuid::parse_str(INFO_UUID).unwrap())
            .await?;
        let val = ch.read().await?;
        Ok(String::from_utf8_lossy(&val).into_owned())
    }

    pub async fn read_slots(&self) -> Result<PadSlots, BleError> {
        let ch = self
            .find_char(
                Uuid::parse_str(SERVICE_UUID).unwrap(),
                Uuid::parse_str(SLOTS_UUID).unwrap(),
            )
            .await?;
        let val = ch.read().await?;
        PadSlots::unpack(&val)
    }

    pub async fn write_slots(&self, slots: &PadSlots) -> Result<(), BleError> {
        let packed = slots.pack()?;
        let ch = self
            .find_char(
                Uuid::parse_str(SERVICE_UUID).unwrap(),
                Uuid::parse_str(SLOTS_UUID).unwrap(),
            )
            .await?;
        ch.write(&packed).await?;
        Ok(())
    }

    pub async fn diagnose_notify(&self, wait_secs: u64) -> Result<Option<Vec<u8>>, BleError> {
        use futures_util::StreamExt;

        self.ensure_connected().await?;
        println!(
            "device {} connected={}",
            redact_ble_address(&self.address.to_string()),
            self.device.is_connected().await?
        );
        let svc = Uuid::parse_str(SERVICE_UUID).unwrap();
        let evt = Uuid::parse_str(MACRO_EVENT_UUID).unwrap();
        for service in self.device.services().await? {
            let su = service.uuid().await?;
            println!("service {su}");
            if su != svc {
                continue;
            }
            for ch in service.characteristics().await? {
                let cu = ch.uuid().await?;
                let flags = ch.flags().await?;
                println!("  char {cu} flags={flags:?}");
                if cu != evt {
                    continue;
                }
                println!(
                    "subscribing to MacroEvent — press a MACRO-mode button within {wait_secs}s…"
                );
                let notify = ch.notify().await?;
                let mut notify = std::pin::pin!(notify);
                match tokio::time::timeout(
                    std::time::Duration::from_secs(wait_secs),
                    notify.as_mut().next(),
                )
                .await
                {
                    Ok(Some(chunk)) => {
                        println!("GOT notify: {chunk:?}");
                        return Ok(Some(chunk));
                    }
                    Ok(None) => {
                        println!("notify stream closed");
                        return Ok(None);
                    }
                    Err(_) => {
                        println!("TIMEOUT — no notification arrived");
                        return Ok(None);
                    }
                }
            }
        }
        Err(BleError::CharMissing(MACRO_EVENT_UUID.into()))
    }

    /// Subscribe to MacroEvent notifications. Returns a stream of events.
    pub async fn subscribe_macro_events(
        &self,
    ) -> Result<tokio::sync::mpsc::Receiver<MacroEvent>, BleError> {
        use futures_util::StreamExt;

        let ch = self
            .find_char(
                Uuid::parse_str(SERVICE_UUID).unwrap(),
                Uuid::parse_str(MACRO_EVENT_UUID).unwrap(),
            )
            .await?;

        let (tx, rx) = tokio::sync::mpsc::channel(32);
        let notify = ch.notify().await?;

        tokio::spawn(async move {
            let mut notify = std::pin::pin!(notify);
            while let Some(chunk) = notify.as_mut().next().await {
                if let Some(ev) = MacroEvent::from_bytes(&chunk) {
                    if tx.send(ev).await.is_err() {
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_roundtrip() {
        let slot = HotkeySlot {
            mode: MODE_MACRO,
            r#mod: 0x01,
            key: 0x28,
            label: "Enter".into(),
        };
        let packed = slot.pack();
        let back = HotkeySlot::unpack(&packed).unwrap();
        assert_eq!(back.mode, MODE_MACRO);
        assert_eq!(back.r#mod, 0x01);
        assert_eq!(back.key, 0x28);
        assert_eq!(back.label, "Enter");
    }

    #[test]
    fn redact_ble_address_keeps_last_octet() {
        assert_eq!(
            redact_ble_address("aa:bb:cc:dd:ee:ff"),
            "**:**:**:**:**:FF"
        );
        assert_eq!(
            redact_ble_address("AA-BB-CC-DD-EE-12"),
            "**:**:**:**:**:12"
        );
        assert_eq!(redact_ble_address(""), "(no address)");
        assert_eq!(redact_ble_address("not-a-mac"), "**:**:**:**:**:**");
        let logged = pad_status_for_log(&PadStatus {
            address: "11:22:33:44:55:66".into(),
            name: Some("Cyberdeck Pad".into()),
            connected: true,
            paired: true,
            info: None,
        });
        assert_eq!(logged.address, "**:**:**:**:**:66");
        assert!(!logged.address.contains("11:22"));
    }

    #[test]
    fn slots_pack_len() {
        let slots = PadSlots {
            slots: (0..SLOT_COUNT)
                .map(|i| HotkeySlot {
                    mode: MODE_HID,
                    r#mod: 0,
                    key: i as u8,
                    label: format!("s{i}"),
                })
                .collect(),
        };
        let packed = slots.pack().unwrap();
        assert_eq!(packed.len(), SLOTS_BYTES);
        let back = PadSlots::unpack(&packed).unwrap();
        assert_eq!(back.slots.len(), SLOT_COUNT);
        assert_eq!(back.slots[3].label, "s3");
    }
}
