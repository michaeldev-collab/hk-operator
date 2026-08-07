//! Write 18 HotkeySlot JSON array to the pad via S3 dongle (USB CDC / serial).

use cyberdeck_ble::HotkeySlot;
use cyberdeck_dongle::DonglePad;
use std::fs;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: write_dongle_slots slots.json");
    let raw = fs::read_to_string(&path)?;
    let slots: Vec<HotkeySlot> = serde_json::from_str(&raw)?;
    if slots.len() != 18 {
        return Err(format!("need 18 slots, got {}", slots.len()).into());
    }

    // Brief settle — MCC may have just released the CDC port.
    thread::sleep(Duration::from_millis(400));

    let mut d = DonglePad::open()?;
    match d.status() {
        Ok(st) => println!(
            "dongle: connected={} transport={:?}",
            st.connected, st.transport
        ),
        Err(e) => {
            eprintln!("status warn: {e} — trying write anyway");
        }
    }
    d.write_slots(0, &slots)?;
    println!("OK wrote {} slots via dongle", slots.len());
    if let Ok(back) = d.read_slots(0) {
        for i in 0..6 {
            let s = &back.slots[i];
            println!(
                "P{} B{}: mod=0x{:02x} key=0x{:02x} {}",
                i / 3 + 1,
                [2, 4, 5][i % 3],
                s.r#mod,
                s.key,
                s.label
            );
        }
    }
    Ok(())
}
