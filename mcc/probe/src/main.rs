//! cyberdeck-probe — list / read / write / listen against bonded Cyberdeck Pad.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use cyberdeck_ble::{
    pad_status_for_log, redact_ble_address, CyberdeckPad, HotkeySlot, PadSlots, MODE_HID,
    MODE_MACRO,
};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "cyberdeck-probe", about = "Probe Cyberdeck Pad hybrid GATT via BlueZ")]
struct Cli {
    /// Bluetooth address (optional; otherwise find by name "Cyberdeck Pad")
    #[arg(long, global = true)]
    address: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Show adapter + pad connection status
    Status,
    /// Read Info characteristic
    Info,
    /// Read and print all 18 slots as JSON
    ReadSlots,
    /// Write slots from a JSON file (array of 18 HotkeySlot objects)
    WriteSlots {
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
    /// Set one slot: --preset 0 --action 0 --mode hid|macro --mod 0x01 --key 0x28 --label Enter
    SetSlot {
        #[arg(long)]
        preset: usize,
        #[arg(long)]
        action: usize,
        #[arg(long, default_value = "hid")]
        mode: String,
        #[arg(long, default_value = "0")]
        r#mod: String,
        #[arg(long, default_value = "0")]
        key: String,
        #[arg(long, default_value = "")]
        label: String,
    },
    /// Subscribe to MacroEvent notifications (Ctrl+C to quit)
    Listen,
    /// Subscribe once and wait for a single MacroEvent (diagnostic)
    DiagNotify {
        #[arg(long, default_value_t = 15)]
        seconds: u64,
    },
}

fn parse_u8(s: &str) -> Result<u8> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Ok(u8::from_str_radix(hex, 16)?)
    } else {
        Ok(t.parse()?)
    }
}

async fn open_pad(address: &Option<String>) -> Result<CyberdeckPad> {
    let (_session, adapter) = CyberdeckPad::session_adapter().await?;
    let pad = if let Some(addr) = address {
        CyberdeckPad::find_by_address(&adapter, addr).await?
    } else {
        CyberdeckPad::find(&adapter).await?
    };
    pad.ensure_connected().await?;
    Ok(pad)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.cmd {
        Cmd::Status => {
            let (_session, adapter) = CyberdeckPad::session_adapter().await?;
            println!("adapter: {}", adapter.name());
            println!("powered: {}", adapter.is_powered().await?);
            match if let Some(addr) = &cli.address {
                CyberdeckPad::find_by_address(&adapter, addr).await
            } else {
                CyberdeckPad::find(&adapter).await
            } {
                Ok(pad) => {
                    let st = pad.status().await?;
                    println!("{}", serde_json::to_string_pretty(&pad_status_for_log(&st))?);
                }
                Err(e) => {
                    println!("pad: not found ({e})");
                    println!("tip: pair the pad as a keyboard in system Bluetooth, then retry");
                }
            }
        }
        Cmd::Info => {
            let pad = open_pad(&cli.address).await?;
            println!("{}", pad.read_info().await?);
        }
        Cmd::ReadSlots => {
            let pad = open_pad(&cli.address).await?;
            let slots = pad.read_slots().await?;
            println!("{}", serde_json::to_string_pretty(&slots.slots)?);
        }
        Cmd::WriteSlots { file } => {
            let pad = open_pad(&cli.address).await?;
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("read {}", file.display()))?;
            let list: Vec<HotkeySlot> = serde_json::from_str(&text)?;
            if list.len() != 18 {
                bail!("expected 18 slots, got {}", list.len());
            }
            let slots = PadSlots { slots: list };
            pad.write_slots(&slots).await?;
            println!("wrote 18 slots ok");
        }
        Cmd::SetSlot {
            preset,
            action,
            mode,
            r#mod,
            key,
            label,
        } => {
            let pad = open_pad(&cli.address).await?;
            let mut slots = pad.read_slots().await?;
            let mode_u8 = match mode.to_lowercase().as_str() {
                "macro" | "1" => MODE_MACRO,
                _ => MODE_HID,
            };
            let slot = slots.get_mut(preset, action)?;
            *slot = HotkeySlot {
                mode: mode_u8,
                r#mod: parse_u8(&r#mod)?,
                key: parse_u8(&key)?,
                label,
            };
            let printed = serde_json::to_string(slot)?;
            pad.write_slots(&slots).await?;
            println!("set preset={preset} action={action} -> {printed}");
        }
        Cmd::Listen => {
            let pad = open_pad(&cli.address).await?;
            println!(
                "listening for MacroEvent on {} … (press a MACRO-mode button)",
                redact_ble_address(&pad.address.to_string())
            );
            let mut rx = pad.subscribe_macro_events().await?;
            while let Some(ev) = rx.recv().await {
                println!(
                    "MacroEvent preset={} action={}  (binding key \"{}-{}\")",
                    ev.preset, ev.action, ev.preset, ev.action
                );
            }
        }
        Cmd::DiagNotify { seconds } => {
            let pad = open_pad(&cli.address).await?;
            let got = pad.diagnose_notify(seconds).await?;
            if got.is_none() {
                bail!("no MacroEvent — firmware may be gating notify, or button not in MACRO mode / wrong preset");
            }
        }
    }

    Ok(())
}
