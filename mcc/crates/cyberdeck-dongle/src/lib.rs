//! USB CDC client for the Cyberpad S3 validation dongle (slots proxy).

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use cyberdeck_ble::{
    info_supports_banks, HotkeySlot, MacroEvent, PadSlots, PadStatus, BANK_COUNT, SLOTS_PAGE_BYTES,
};
use rusb::{Context, DeviceHandle, Direction, TransferType, UsbContext};
use std::io::{Read, Write};
use std::time::Duration;
use thiserror::Error;

const VID: u16 = 0x303a;
const PID: u16 = 0x1001;
const S3_SERIAL_COMPACT: &str = "A0F262F3D5CC";
const TIMEOUT: Duration = Duration::from_millis(2000);
const MAX_RESPONSE_BYTES: usize = 8192;
const SLOTS_B64_BYTES: usize = 652;
pub const EXPECTED_DONGLE_VERSION: &str = "s3-dongle-validation 0.5.3 protocol-v0.3";

#[derive(Debug, Error)]
pub enum DongleError {
    #[error("usb: {0}")]
    Usb(#[from] rusb::Error),
    #[error("serial: {0}")]
    Serial(String),
    #[error("dongle not found")]
    NotFound,
    #[error("dongle present but unavailable: {0}")]
    Unavailable(String),
    #[error("protocol: {0}")]
    Proto(String),
    #[error("{0}")]
    Msg(String),
}

pub struct DonglePad {
    kind: Transport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DonglePoll {
    pub selected_bank: u8,
    pub macro_event: Option<MacroEvent>,
}

enum Transport {
    Serial(Box<dyn serialport::SerialPort>),
    UsbCdc {
        handle: DeviceHandle<Context>,
        out_ep: u8,
        in_ep: u8,
    },
}

impl DonglePad {
    /// Prefer `/dev/ttyACM*` if present; otherwise userspace TinyUSB CDC via libusb.
    pub fn open() -> Result<Self, DongleError> {
        let serial_error = match open_serial_acm() {
            Ok(port) => {
                return Ok(Self {
                    kind: Transport::Serial(port),
                })
            }
            Err(DongleError::NotFound) => None,
            Err(error) => Some(error),
        };
        match open_usb_cdc() {
            Ok(cdc) => Ok(Self {
                kind: Transport::UsbCdc {
                    handle: cdc.handle,
                    out_ep: cdc.out_ep,
                    in_ep: cdc.in_ep,
                },
            }),
            Err(DongleError::NotFound) => {
                if let Some(error) = serial_error {
                    Err(error)
                } else if s3_present_in_sysfs() {
                    Err(DongleError::Unavailable(
                        "exact USB identity exists in sysfs but neither serial nor libusb could open it"
                            .into(),
                    ))
                } else {
                    Err(DongleError::NotFound)
                }
            }
            Err(error) => Err(error),
        }
    }

    pub fn available() -> bool {
        Self::open().is_ok()
    }

    /// True when dongle USB is present and BLE reports connected + slots ready.
    pub fn linked_for_slots() -> bool {
        match Self::open() {
            Ok(mut d) => d
                .status()
                .map(|s| {
                    s.connected
                        && s.slots_ready == Some(true)
                        && s.protocol_compatible == Some(true)
                })
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// True when the v0.3 dongle reports a live MacroEvent subscription.
    pub fn linked_for_macros() -> bool {
        match Self::open() {
            Ok(mut d) => d
                .status()
                .map(|s| {
                    s.connected
                        && s.macro_ready == Some(true)
                        && s.protocol_compatible == Some(true)
                })
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Query the running S3 application and require the exact release build string.
    ///
    /// A matching `protocol=v0.3` status token is not sufficient: older or
    /// locally modified dongle firmware can expose the same protocol surface.
    pub fn version(&mut self) -> Result<&'static str, DongleError> {
        let response = self.cmd_line("version")?;
        validate_version_response(&response)?;
        Ok(EXPECTED_DONGLE_VERSION)
    }

    pub fn status(&mut self) -> Result<PadStatus, DongleError> {
        self.version()?;
        let line = self.status_line()?;
        let info = self.cmd_line("pad info").ok().and_then(|resp| {
            resp.lines()
                .find_map(|l| l.strip_prefix("INFO ").map(|s| s.to_string()))
        });
        let protocol_compatible =
            line.protocol_v03 && info.as_deref().is_some_and(info_supports_banks);
        Ok(PadStatus {
            address: "via-s3-dongle".into(),
            name: Some("Cyberpad Val C6".into()),
            connected: line.ble_connected,
            paired: true,
            info,
            transport: Some("dongle".into()),
            bluez_blocked: None,
            protocol_compatible: Some(protocol_compatible),
            slots_ready: Some(line.slots_ready),
            macro_ready: Some(line.macro_ready),
            selected_bank: Some(line.selected_bank),
        })
    }

    pub fn read_slots(&mut self, bank: u8) -> Result<PadSlots, DongleError> {
        validate_bank(bank)?;
        let resp = self.cmd_line(&format!("slots read {bank}"))?;
        let prefix = format!("SLOTS {bank} ");
        let b64 = resp
            .lines()
            .find_map(|l| l.strip_prefix(&prefix).map(|s| s.trim().to_string()))
            .ok_or_else(|| DongleError::Proto(format!("no bank {bank} SLOTS line in: {resp}")))?;
        if b64.len() != SLOTS_B64_BYTES || !b64.ends_with("==") {
            return Err(DongleError::Proto(format!(
                "bank {bank} SLOTS base64 length/padding is not canonical"
            )));
        }
        let raw = B64
            .decode(b64.as_bytes())
            .map_err(|e| DongleError::Proto(format!("b64: {e}")))?;
        PadSlots::unpack_bank(&raw, bank).map_err(|e| DongleError::Msg(e.to_string()))
    }

    pub fn write_slots(&mut self, bank: u8, slots: &[HotkeySlot]) -> Result<(), DongleError> {
        validate_bank(bank)?;
        let packed = PadSlots {
            slots: slots.to_vec(),
        }
        .pack_bank(bank)
        .map_err(|e| DongleError::Msg(e.to_string()))?;
        if packed.len() != SLOTS_PAGE_BYTES {
            return Err(DongleError::Proto(format!(
                "packed {} want {SLOTS_PAGE_BYTES}",
                packed.len()
            )));
        }
        let b64 = B64.encode(packed);
        let resp = self.cmd_line(&format!("slots write {bank} {b64}"))?;
        if resp
            .lines()
            .any(|l| l.trim() == format!("OK bank={bank} verified"))
        {
            Ok(())
        } else {
            Err(DongleError::Proto(format!("write failed: {resp}")))
        }
    }

    pub fn poll_events(&mut self) -> Result<DonglePoll, DongleError> {
        let resp = self.cmd_line("macro next")?;
        let selected_bank = resp
            .lines()
            .find_map(|line| line.strip_prefix("BANK "))
            .and_then(|value| value.trim().parse::<u8>().ok())
            .filter(|bank| (*bank as usize) < BANK_COUNT)
            .ok_or_else(|| DongleError::Proto(format!("no valid BANK line in: {resp}")))?;
        for line in resp.lines() {
            let Some(rest) = line.strip_prefix("MACRO ") else {
                continue;
            };
            let values = rest
                .split_whitespace()
                .map(str::parse::<u8>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| DongleError::Proto(format!("bad macro event: {e}")))?;
            if values.len() != 3 {
                return Err(DongleError::Proto(format!("bad macro event: {line}")));
            }
            let macro_event = MacroEvent::from_bytes(&values)
                .ok_or_else(|| DongleError::Proto(format!("invalid macro event: {line}")))?;
            return Ok(DonglePoll {
                selected_bank,
                macro_event: Some(macro_event),
            });
        }
        if resp.lines().any(|l| l.trim() == "NONE") {
            return Ok(DonglePoll {
                selected_bank,
                macro_event: None,
            });
        }
        Err(DongleError::Proto(format!("no macro response in: {resp}")))
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
        StatusBits::parse(&line)
    }

    fn cmd_line(&mut self, cmd: &str) -> Result<String, DongleError> {
        // A prior timeout may leave a complete old reply queued in CDC. Drain
        // before issuing the next command so stale terminal records cannot
        // acknowledge a newer bank write. Request IDs remain the stronger
        // future protocol upgrade, but this closes the current retry path.
        self.clear_input()?;
        self.write_all(&(cmd.to_string() + "\n"))?;
        std::thread::sleep(Duration::from_millis(10));
        self.read_until_idle(cmd, Duration::from_millis(1500))
    }

    fn clear_input(&mut self) -> Result<(), DongleError> {
        match &mut self.kind {
            Transport::Serial(port) => port
                .clear(serialport::ClearBuffer::Input)
                .map_err(|error| DongleError::Serial(error.to_string())),
            Transport::UsbCdc { handle, in_ep, .. } => {
                let mut discarded = 0usize;
                loop {
                    let mut chunk = [0u8; 256];
                    match handle.read_bulk(*in_ep, &mut chunk, Duration::from_millis(2)) {
                        Ok(0) | Err(rusb::Error::Timeout) => return Ok(()),
                        Ok(count) => {
                            discarded += count;
                            if discarded > MAX_RESPONSE_BYTES {
                                return Err(DongleError::Proto(format!(
                                    "CDC stale input exceeds {MAX_RESPONSE_BYTES} bytes"
                                )));
                            }
                        }
                        Err(error) => return Err(DongleError::Usb(error)),
                    }
                }
            }
        }
    }

    fn write_all(&mut self, data: &str) -> Result<(), DongleError> {
        match &mut self.kind {
            Transport::Serial(p) => {
                p.write_all(data.as_bytes())
                    .map_err(|e| DongleError::Serial(e.to_string()))?;
                p.flush().map_err(|e| DongleError::Serial(e.to_string()))?;
            }
            Transport::UsbCdc { handle, out_ep, .. } => {
                let mut remaining = data.as_bytes();
                while !remaining.is_empty() {
                    let n = handle
                        .write_bulk(*out_ep, remaining, TIMEOUT)
                        .map_err(DongleError::Usb)?;
                    if n == 0 {
                        return Err(DongleError::Proto("zero-length USB CDC write".into()));
                    }
                    remaining = &remaining[n..];
                }
            }
        }
        Ok(())
    }

    fn read_until_idle(&mut self, cmd: &str, overall: Duration) -> Result<String, DongleError> {
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
                if buf.len() + n > MAX_RESPONSE_BYTES {
                    return Err(DongleError::Proto(format!(
                        "CDC response exceeds {MAX_RESPONSE_BYTES} bytes"
                    )));
                }
                buf.extend_from_slice(&chunk[..n]);
                if response_complete(cmd, &buf) {
                    break;
                }
                idle_rounds = 0;
            } else {
                idle_rounds += 1;
                // The idle early-exit exists to stop quickly once a reply has
                // landed. Async bracketed chatter ("[bank] selected N" racing a
                // slots read) arrives instantly, while the real record can take
                // a BLE long-read's time -- so chatter alone must NOT satisfy
                // the exit, or ~1 in 5 bank reads returns before its SLOTS
                // line. Only give up early once a terminated, non-bracketed
                // record line exists; otherwise wait out `overall`.
                if has_record_line(&buf) && idle_rounds >= 3 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

/// True when the buffer holds at least one dongle record: a `\n`-terminated
/// line that is not empty and not `[...]` async chatter. Every real reply
/// (ERR/SLOTS/OK/INFO/BANK/MACRO/NONE/version/status) matches; chatter never
/// does.
fn has_record_line(bytes: &[u8]) -> bool {
    let Some(last_newline) = bytes.iter().rposition(|byte| *byte == b'\n') else {
        return false;
    };
    String::from_utf8_lossy(&bytes[..=last_newline])
        .lines()
        .any(|line| !line.trim().is_empty() && !line.starts_with('['))
}

fn response_complete(cmd: &str, bytes: &[u8]) -> bool {
    // Only parse records that the dongle has terminated. `str::lines()` also
    // yields an unterminated final fragment, which used to make the first
    // 256-byte chunk of a long SLOTS response look complete.
    let Some(last_newline) = bytes.iter().rposition(|byte| *byte == b'\n') else {
        return false;
    };
    let text = String::from_utf8_lossy(&bytes[..=last_newline]);
    let lines: Vec<_> = text.lines().collect();
    let has_err = lines.iter().any(|line| line.starts_with("ERR "));
    if has_err {
        return true;
    }
    if cmd == "status" {
        return lines.iter().any(|line| line.contains("ble_connected="));
    }
    if cmd == "version" {
        return lines
            .iter()
            .any(|line| line.trim().starts_with("s3-dongle-validation "));
    }
    if cmd == "pad info" {
        return lines.iter().any(|line| line.starts_with("INFO "));
    }
    if cmd == "macro next" {
        let bank = lines.iter().any(|line| line.starts_with("BANK "));
        let event = lines
            .iter()
            .any(|line| line.starts_with("MACRO ") || line.trim() == "NONE");
        return bank && event;
    }
    if let Some(bank) = cmd.strip_prefix("slots read ") {
        let prefix = format!("SLOTS {bank} ");
        return lines.iter().any(|line| {
            line.strip_prefix(&prefix)
                .is_some_and(|b64| b64.len() == SLOTS_B64_BYTES && b64.ends_with("=="))
        });
    }
    if let Some(rest) = cmd.strip_prefix("slots write ") {
        let bank = rest.split_whitespace().next().unwrap_or_default();
        let expected = format!("OK bank={bank} verified");
        return lines.iter().any(|line| line.trim() == expected);
    }
    false
}

fn validate_version_response(response: &str) -> Result<(), DongleError> {
    let versions: Vec<_> = response
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("s3-dongle-validation "))
        .collect();
    if versions.is_empty() {
        return Err(DongleError::Proto(format!(
            "missing dongle version record; expected {EXPECTED_DONGLE_VERSION:?} in: {response}"
        )));
    }
    if let Some(actual) = versions
        .iter()
        .find(|version| **version != EXPECTED_DONGLE_VERSION)
    {
        return Err(DongleError::Proto(format!(
            "unsupported dongle version {actual:?}; expected {EXPECTED_DONGLE_VERSION:?}"
        )));
    }
    Ok(())
}

struct StatusBits {
    ble_connected: bool,
    slots_ready: bool,
    macro_ready: bool,
    protocol_v03: bool,
    selected_bank: u8,
}

fn status_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .find_map(|token| token.strip_prefix(key)?.strip_prefix('='))
}

fn status_bool(line: &str, key: &str) -> Option<bool> {
    match status_value(line, key)? {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

impl StatusBits {
    fn parse(line: &str) -> Result<Self, DongleError> {
        let ble_connected = status_bool(line, "ble_connected").ok_or_else(|| {
            DongleError::Proto(format!("invalid/missing ble_connected in status: {line}"))
        })?;
        let protocol_v03 = status_value(line, "protocol") == Some("v0.3");
        if !protocol_v03 {
            // v0.2 did not report macro_ready/protocol/bank. Preserve the fact
            // that it owns the BLE link so callers can fail closed on mismatch.
            return Ok(Self {
                ble_connected,
                slots_ready: status_bool(line, "slots_ready").unwrap_or(false),
                macro_ready: false,
                protocol_v03: false,
                selected_bank: 0,
            });
        }
        let selected_bank = status_value(line, "bank")
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|bank| (*bank as usize) < BANK_COUNT)
            .ok_or_else(|| DongleError::Proto(format!("invalid/missing bank in status: {line}")))?;
        Ok(Self {
            ble_connected,
            slots_ready: status_bool(line, "slots_ready").ok_or_else(|| {
                DongleError::Proto(format!("invalid/missing slots_ready in status: {line}"))
            })?,
            macro_ready: status_bool(line, "macro_ready").ok_or_else(|| {
                DongleError::Proto(format!("invalid/missing macro_ready in status: {line}"))
            })?,
            protocol_v03,
            selected_bank,
        })
    }
}

fn validate_bank(bank: u8) -> Result<(), DongleError> {
    if bank as usize >= BANK_COUNT {
        return Err(DongleError::Proto(format!(
            "bank {bank} outside 0..{}",
            BANK_COUNT - 1
        )));
    }
    Ok(())
}

fn open_serial_acm() -> Result<Box<dyn serialport::SerialPort>, DongleError> {
    let ports = serialport::available_ports().map_err(|e| DongleError::Serial(e.to_string()))?;
    // Require the exact S3 serial. The C6 pad exposes the same Espressif VID/PID
    // and generic product string, so a VID-only fallback can open the wrong board.
    let mut candidates: Vec<(i32, String)> = Vec::new();
    for p in ports {
        let name = p.port_name;
        if !(name.contains("ttyACM") || name.contains("ttyUSB")) {
            continue;
        }
        let mut score = 0;
        if let serialport::SerialPortType::UsbPort(info) = &p.port_type {
            let ser = info.serial_number.as_deref().unwrap_or("");
            let compact: String = ser
                .chars()
                .filter(|c| c.is_ascii_hexdigit())
                .map(|c| c.to_ascii_uppercase())
                .collect();
            if info.vid == VID && info.pid == PID && compact == S3_SERIAL_COMPACT {
                score = 100;
            }
        }
        candidates.push((score, name));
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let mut exact_open_error = None;
    for (score, name) in candidates {
        if score <= 0 {
            continue;
        }
        match serialport::new(&name, 115_200)
            .timeout(Duration::from_millis(200))
            .open()
        {
            Ok(port) => return Ok(port),
            Err(e) => {
                exact_open_error = Some(format!("open {name}: {e}"));
            }
        }
    }
    match exact_open_error {
        Some(error) => Err(DongleError::Unavailable(error)),
        None => Err(DongleError::NotFound),
    }
}

fn s3_present_in_sysfs() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let base = entry.path();
        let read_trimmed = |name: &str| {
            std::fs::read_to_string(base.join(name))
                .ok()
                .map(|value| value.trim().to_string())
        };
        if read_trimmed("idVendor").as_deref() != Some("303a")
            || read_trimmed("idProduct").as_deref() != Some("1001")
        {
            return false;
        }
        read_trimmed("serial").is_some_and(|serial| {
            serial
                .chars()
                .filter(|character| character.is_ascii_hexdigit())
                .map(|character| character.to_ascii_uppercase())
                .collect::<String>()
                == S3_SERIAL_COMPACT
        })
    })
}

struct UsbCdc {
    handle: DeviceHandle<Context>,
    out_ep: u8,
    in_ep: u8,
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
        if ser.eq_ignore_ascii_case(S3_SERIAL_COMPACT) {
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

    // libusb now owns detach/reattach for every claimed CDC interface. This
    // guarantees the kernel driver is restored when the cached handle drops.
    handle.set_auto_detach_kernel_driver(true)?;

    if let Some(iface) = comm_iface {
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

    handle.claim_interface(data_iface)?;

    Ok(UsbCdc {
        handle,
        out_ep,
        in_ep,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_v03_status_tokens() {
        let status = StatusBits::parse(
            "state=CONNECTED ble_connected=1 have_peer=1 slots_ready=1 macro_ready=1 bank=4 protocol=v0.3",
        )
        .unwrap();
        assert!(status.ble_connected);
        assert!(status.slots_ready);
        assert!(status.macro_ready);
        assert!(status.protocol_v03);
        assert_eq!(status.selected_bank, 4);
    }

    #[test]
    fn legacy_status_remains_visible_as_incompatible() {
        let status =
            StatusBits::parse("state=CONNECTED ble_connected=1 have_peer=1 slots_ready=1").unwrap();
        assert!(status.ble_connected);
        assert!(!status.protocol_v03);
        assert!(!status.macro_ready);
        assert_eq!(status.selected_bank, 0);
    }

    #[test]
    fn status_parser_rejects_substring_boolean_values() {
        assert!(StatusBits::parse("ble_connected=10 protocol=v0.2").is_err());
    }

    #[test]
    fn macro_response_requires_bank_and_terminal_line() {
        assert!(!response_complete("macro next", b"BANK 2\n"));
        assert!(!response_complete("macro next", b"BANK 2\nNON"));
        assert!(response_complete("macro next", b"BANK 2\nNONE\n"));
        assert!(response_complete("macro next", b"BANK 2\nMACRO 2 1 0\n"));
    }

    #[test]
    fn async_chatter_alone_is_not_a_record() {
        // "[bank] selected N" races a slots read; it must keep the reader
        // waiting instead of satisfying the idle early-exit.
        assert!(!has_record_line(b""));
        assert!(!has_record_line(b"[bank] selected 2\n"));
        assert!(!has_record_line(b"[bank] selected 2\n[ble] HEARTBEAT seq=9\n"));
        assert!(!has_record_line(b"SLOTS 3 unterminated-fragment"));
        assert!(has_record_line(b"[bank] selected 2\nERR slots not ready\n"));
        assert!(has_record_line(b"OK bank=3 verified\n"));
    }

    #[test]
    fn fragmented_terminal_lines_are_not_complete() {
        let partial_slots = format!("SLOTS 2 {}", "A".repeat(240));
        assert!(!response_complete("slots read 2", partial_slots.as_bytes()));
        let full_slots = format!("SLOTS 2 {}==\n", "A".repeat(SLOTS_B64_BYTES - 2));
        assert!(response_complete("slots read 2", full_slots.as_bytes()));
        assert!(!response_complete("slots read 3", full_slots.as_bytes()));
        assert!(!response_complete("status", b"ble_connected=1"));
        assert!(response_complete("status", b"ble_connected=1\n"));
        assert!(!response_complete("pad info", b"ERR unavailable"));
        assert!(response_complete("pad info", b"ERR unavailable\n"));
        assert!(!response_complete(
            "version",
            b"s3-dongle-validation 0.5.3 protocol-v0.3"
        ));
        assert!(response_complete(
            "version",
            b"s3-dongle-validation 0.5.3 protocol-v0.3\n"
        ));
        assert!(response_complete(
            "version",
            b"s3-dongle-validation 0.4.4 protocol-v0.2\n"
        ));
    }

    #[test]
    fn exact_release_version_is_required() {
        validate_version_response("s3-dongle-validation 0.5.3 protocol-v0.3\r\n").unwrap();
        validate_version_response("[ble] diagnostic\ns3-dongle-validation 0.5.3 protocol-v0.3\n")
            .unwrap();

        for response in [
            "s3-dongle-validation 0.4.4 protocol-v0.2\n",
            "s3-dongle-validation 0.5.0 protocol-v0.3\n",
            "s3-dongle-validation 0.5.3 protocol-v0.3-extra\n",
            "prefix s3-dongle-validation 0.5.3 protocol-v0.3\n",
            "unrelated diagnostics\n",
        ] {
            assert!(
                validate_version_response(response).is_err(),
                "accepted {response:?}"
            );
        }
    }
}
