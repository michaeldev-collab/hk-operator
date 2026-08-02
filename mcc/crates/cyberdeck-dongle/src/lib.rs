//! USB CDC client for the Cyberpad S3 validation dongle (slots proxy).

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use cyberdeck_ble::{HotkeySlot, PadSlots, PadStatus, SLOTS_BYTES};
use rusb::{Context, DeviceHandle, Direction, TransferType, UsbContext};
use std::io::{Read, Write};
use std::time::Duration;
use thiserror::Error;

const VID: u16 = 0x303a;
const PID: u16 = 0x1001;
const S3_SERIAL_COMPACT: &str = "A0F262F3D5CC";
const TIMEOUT: Duration = Duration::from_millis(2000);

#[derive(Debug, Error)]
pub enum DongleError {
    #[error("usb: {0}")]
    Usb(#[from] rusb::Error),
    #[error("serial: {0}")]
    Serial(String),
    #[error("dongle not found")]
    NotFound,
    #[error("protocol: {0}")]
    Proto(String),
    #[error("{0}")]
    Msg(String),
}

pub struct DonglePad {
    kind: Transport,
}

enum Transport {
    Serial(Box<dyn serialport::SerialPort>),
    UsbCdc {
        handle: DeviceHandle<Context>,
        out_ep: u8,
        in_ep: u8,
        #[allow(dead_code)]
        data_iface: u8,
    },
}

impl DonglePad {
    /// Prefer `/dev/ttyACM*` if present; otherwise userspace TinyUSB CDC via libusb.
    pub fn open() -> Result<Self, DongleError> {
        if let Ok(port) = open_serial_acm() {
            return Ok(Self {
                kind: Transport::Serial(port),
            });
        }
        let cdc = open_usb_cdc()?;
        Ok(Self {
            kind: Transport::UsbCdc {
                handle: cdc.handle,
                out_ep: cdc.out_ep,
                in_ep: cdc.in_ep,
                data_iface: cdc.data_iface,
            },
        })
    }

    pub fn available() -> bool {
        Self::open().is_ok()
    }

    /// True when dongle USB is present and BLE reports connected + slots ready.
    pub fn linked_for_slots() -> bool {
        match Self::open() {
            Ok(mut d) => d
                .status_line()
                .map(|s| s.ble_connected && s.slots_ready)
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    pub fn status(&mut self) -> Result<PadStatus, DongleError> {
        let line = self.status_line()?;
        let info = self.cmd_line("pad info").ok().and_then(|resp| {
            resp.lines()
                .find_map(|l| l.strip_prefix("INFO ").map(|s| s.to_string()))
        });
        Ok(PadStatus {
            address: "via-s3-dongle".into(),
            name: Some("Cyberpad Val C6".into()),
            connected: line.ble_connected,
            paired: true,
            info,
            transport: Some("dongle".into()),
            bluez_blocked: None,
        })
    }

    pub fn read_slots(&mut self) -> Result<PadSlots, DongleError> {
        let resp = self.cmd_line("slots read")?;
        let b64 = resp
            .lines()
            .find_map(|l| l.strip_prefix("SLOTS ").map(|s| s.trim().to_string()))
            .ok_or_else(|| DongleError::Proto(format!("no SLOTS line in: {resp}")))?;
        let raw = B64
            .decode(b64.as_bytes())
            .map_err(|e| DongleError::Proto(format!("b64: {e}")))?;
        PadSlots::unpack(&raw).map_err(|e| DongleError::Msg(e.to_string()))
    }

    pub fn write_slots(&mut self, slots: &[HotkeySlot]) -> Result<(), DongleError> {
        let packed = PadSlots {
            slots: slots.to_vec(),
        }
        .pack()
        .map_err(|e| DongleError::Msg(e.to_string()))?;
        if packed.len() != SLOTS_BYTES {
            return Err(DongleError::Proto(format!(
                "packed {} want {SLOTS_BYTES}",
                packed.len()
            )));
        }
        let b64 = B64.encode(packed);
        let resp = self.cmd_line(&format!("slots write {b64}"))?;
        if resp.lines().any(|l| l.trim() == "OK") {
            Ok(())
        } else {
            Err(DongleError::Proto(format!("write failed: {resp}")))
        }
    }

    fn status_line(&mut self) -> Result<StatusBits, DongleError> {
        let resp = self.cmd_line("status")?;
        let line = resp
            .lines()
            .find(|l| l.contains("ble_connected="))
            .unwrap_or("")
            .to_string();
        if line.is_empty() {
            return Err(DongleError::Proto(format!("no status line in: {resp}")));
        }
        Ok(StatusBits {
            ble_connected: line.contains("ble_connected=1"),
            slots_ready: line.contains("slots_ready=1"),
        })
    }

    fn cmd_line(&mut self, cmd: &str) -> Result<String, DongleError> {
        self.write_all(&(cmd.to_string() + "\n"))?;
        std::thread::sleep(Duration::from_millis(80));
        self.read_until_idle(Duration::from_millis(1500))
    }

    fn write_all(&mut self, data: &str) -> Result<(), DongleError> {
        match &mut self.kind {
            Transport::Serial(p) => {
                p.write_all(data.as_bytes())
                    .map_err(|e| DongleError::Serial(e.to_string()))?;
                p.flush()
                    .map_err(|e| DongleError::Serial(e.to_string()))?;
            }
            Transport::UsbCdc {
                handle, out_ep, ..
            } => {
                let mut remaining = data.as_bytes();
                while !remaining.is_empty() {
                    let n = handle
                        .write_bulk(*out_ep, remaining, TIMEOUT)
                        .map_err(DongleError::Usb)?;
                    remaining = &remaining[n..];
                }
            }
        }
        Ok(())
    }

    fn read_until_idle(&mut self, overall: Duration) -> Result<String, DongleError> {
        let deadline = std::time::Instant::now() + overall;
        let mut buf = Vec::new();
        let mut idle_rounds = 0u32;
        while std::time::Instant::now() < deadline {
            let mut chunk = [0u8; 256];
            let n = match &mut self.kind {
                Transport::Serial(p) => match p.read(&mut chunk) {
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => 0,
                    Err(e) => return Err(DongleError::Serial(e.to_string())),
                },
                Transport::UsbCdc { handle, in_ep, .. } => {
                    match handle.read_bulk(*in_ep, &mut chunk, Duration::from_millis(80)) {
                        Ok(n) => n,
                        Err(rusb::Error::Timeout) => 0,
                        Err(e) => return Err(DongleError::Usb(e)),
                    }
                }
            };
            if n > 0 {
                buf.extend_from_slice(&chunk[..n]);
                idle_rounds = 0;
            } else {
                idle_rounds += 1;
                if !buf.is_empty() && idle_rounds >= 3 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

struct StatusBits {
    ble_connected: bool,
    slots_ready: bool,
}

fn open_serial_acm() -> Result<Box<dyn serialport::SerialPort>, DongleError> {
    let ports = serialport::available_ports().map_err(|e| DongleError::Serial(e.to_string()))?;
    // Prefer Espressif S3 CDC; other ACM devices (e.g. CH343) speak a different protocol.
    let mut candidates: Vec<(i32, String)> = Vec::new();
    for p in ports {
        let name = p.port_name;
        if !(name.contains("ttyACM") || name.contains("ttyUSB")) {
            continue;
        }
        let mut score = 0;
        if let serialport::SerialPortType::UsbPort(info) = &p.port_type {
            let prod = info.product.as_deref().unwrap_or("");
            let ser = info.serial_number.as_deref().unwrap_or("");
            if prod.contains("ESP32S3") || ser.contains(S3_SERIAL_COMPACT) {
                score = 100;
            } else if info.vid == VID {
                score = 50;
            }
        }
        candidates.push((score, name));
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    for (score, name) in candidates {
        if score <= 0 {
            continue;
        }
        match serialport::new(&name, 115_200)
            .timeout(Duration::from_millis(200))
            .open()
        {
            Ok(port) => return Ok(port),
            Err(e) => eprintln!("[dongle] skip {name}: {e}"),
        }
    }
    Err(DongleError::NotFound)
}

struct UsbCdc {
    handle: DeviceHandle<Context>,
    out_ep: u8,
    in_ep: u8,
    data_iface: u8,
}

fn open_usb_cdc() -> Result<UsbCdc, DongleError> {
    let ctx = Context::new()?;
    let mut found = None;
    for device in ctx.devices()?.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        if desc.vendor_id() != VID || desc.product_id() != PID {
            continue;
        }
        let handle = match device.open() {
            Ok(h) => h,
            Err(_) => continue,
        };
        let ser = handle
            .read_serial_number_string_ascii(&desc)
            .unwrap_or_default()
            .replace(':', "");
        let prod = handle
            .read_product_string_ascii(&desc)
            .unwrap_or_default();
        if prod.contains("ESP32S3") || ser.contains(S3_SERIAL_COMPACT) {
            found = Some((device, handle));
            break;
        }
    }
    let (device, handle) = found.ok_or(DongleError::NotFound)?;
    let config = device.active_config_descriptor()?;

    let mut comm_iface = None;
    let mut data_iface = None;
    let mut out_ep = None;
    let mut in_ep = None;

    for interface in config.interfaces() {
        for alt in interface.descriptors() {
            match alt.class_code() {
                2 => {
                    // CDC communication
                    comm_iface = Some(alt.interface_number());
                }
                10 => {
                    // CDC data
                    data_iface = Some(alt.interface_number());
                    for ep in alt.endpoint_descriptors() {
                        if ep.transfer_type() != TransferType::Bulk {
                            continue;
                        }
                        match ep.direction() {
                            Direction::Out => out_ep = Some(ep.address()),
                            Direction::In => in_ep = Some(ep.address()),
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let data_iface = data_iface.ok_or(DongleError::Msg("no CDC data iface".into()))?;
    let out_ep = out_ep.ok_or(DongleError::Msg("no CDC OUT ep".into()))?;
    let in_ep = in_ep.ok_or(DongleError::Msg("no CDC IN ep".into()))?;

    if let Some(iface) = comm_iface {
        let _ = handle.detach_kernel_driver(iface);
        handle.claim_interface(iface)?;
        // SET_LINE_CODING 115200 8N1
        let line = [
            0x00u8, 0xC2, 0x01, 0x00, // 115200 LE
            0x00, 0x00, 0x08,
        ];
        let _ = handle.write_control(
            rusb::request_type(
                rusb::Direction::Out,
                rusb::RequestType::Class,
                rusb::Recipient::Interface,
            ),
            0x20,
            0,
            u16::from(iface),
            &line,
            TIMEOUT,
        );
        // SET_CONTROL_LINE_STATE DTR|RTS
        let _ = handle.write_control(
            rusb::request_type(
                rusb::Direction::Out,
                rusb::RequestType::Class,
                rusb::Recipient::Interface,
            ),
            0x22,
            0x0003,
            u16::from(iface),
            &[],
            TIMEOUT,
        );
    }

    let _ = handle.detach_kernel_driver(data_iface);
    handle.claim_interface(data_iface)?;

    Ok(UsbCdc {
        handle,
        out_ep,
        in_ep,
        data_iface,
    })
}
