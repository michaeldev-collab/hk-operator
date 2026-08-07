use cyberdeck_ble::{PadStatus, BANK_COUNT, SLOT_COUNT};
use cyberdeck_dongle::{DonglePad, EXPECTED_DONGLE_VERSION};

const EXPECTED_PAD_INFO: &str = "Cyberdeck Pad Hybrid v0.3.1";

fn require_pair_ready(status: &PadStatus, phase: &str) -> Result<u8, String> {
    if !status.connected {
        return Err(format!("{phase}: S3 is not connected to the C6"));
    }
    if status.info.as_deref() != Some(EXPECTED_PAD_INFO) {
        return Err(format!(
            "{phase}: pad info mismatch: got {:?}, expected {EXPECTED_PAD_INFO:?}",
            status.info
        ));
    }
    if status.protocol_compatible != Some(true) {
        return Err(format!(
            "{phase}: protocol is not v0.3-compatible: {:?}",
            status.protocol_compatible
        ));
    }
    if status.slots_ready != Some(true) {
        return Err(format!(
            "{phase}: slots subscription is not ready: {:?}",
            status.slots_ready
        ));
    }
    if status.macro_ready != Some(true) {
        return Err(format!(
            "{phase}: macro subscription is not ready: {:?}",
            status.macro_ready
        ));
    }
    status
        .selected_bank
        .filter(|bank| (*bank as usize) < BANK_COUNT)
        .ok_or_else(|| format!("{phase}: selected bank is missing or out of range"))
}

fn verify_version(dongle: &mut DonglePad) -> Result<&'static str, String> {
    let version = dongle.version().map_err(|error| error.to_string())?;
    if version != EXPECTED_DONGLE_VERSION {
        return Err(format!(
            "internal version mismatch: got {version:?}, expected {EXPECTED_DONGLE_VERSION:?}"
        ));
    }
    Ok(EXPECTED_DONGLE_VERSION)
}

fn read_all_bank_pages(dongle: &mut DonglePad) -> Result<usize, String> {
    let mut total_slots = 0usize;
    for bank in 0..BANK_COUNT {
        let slots = dongle
            .read_slots(bank as u8)
            .map_err(|error| format!("bank {bank}: {error}"))?;
        if slots.slots.len() != SLOT_COUNT {
            return Err(format!(
                "bank {bank}: got {} slots, expected {SLOT_COUNT}",
                slots.slots.len()
            ));
        }
        total_slots += slots.slots.len();
        println!("bank={bank} canonical_slots={}", slots.slots.len());
    }
    Ok(total_slots)
}

fn restore_bank(dongle: &mut DonglePad, bank: u8) -> Result<(), String> {
    let slots = dongle
        .read_slots(bank)
        .map_err(|error| format!("could not restore initial bank {bank}: {error}"))?;
    if slots.slots.len() != SLOT_COUNT {
        return Err(format!(
            "could not restore initial bank {bank}: got {} slots, expected {SLOT_COUNT}",
            slots.slots.len()
        ));
    }
    Ok(())
}

fn verify_banks(dongle: &mut DonglePad) -> Result<(), String> {
    let version = verify_version(dongle)?;
    println!("version={version}");
    let initial = dongle.status().map_err(|error| error.to_string())?;
    let initial_bank = require_pair_ready(&initial, "initial status")?;
    println!(
        "pad_info={} connected=1 protocol=v0.3 slots_ready=1 macro_ready=1 initial_bank={initial_bank}",
        initial.info.as_deref().unwrap_or_default()
    );

    let sweep = read_all_bank_pages(dongle);
    let restore = restore_bank(dongle, initial_bank);
    let total_slots = match (sweep, restore) {
        (Ok(total_slots), Ok(())) => total_slots,
        (Err(sweep_error), Ok(())) => return Err(sweep_error),
        (Ok(_), Err(restore_error)) => return Err(restore_error),
        (Err(sweep_error), Err(restore_error)) => {
            return Err(format!("{sweep_error}; additionally, {restore_error}"));
        }
    };

    let final_status = dongle.status().map_err(|error| error.to_string())?;
    let final_bank = require_pair_ready(&final_status, "final status")?;
    if final_bank != initial_bank {
        return Err(format!(
            "final status: bank restoration mismatch: got {final_bank}, expected {initial_bank}"
        ));
    }
    println!(
        "BANK_PAGES_OK banks={BANK_COUNT} canonical_slots={total_slots} restored_bank={final_bank}"
    );
    Ok(())
}

fn run() -> Result<(), String> {
    let mut args = std::env::args();
    let program = args
        .next()
        .unwrap_or_else(|| "cyberdeck-dongle-verify".to_string());
    let command = match (args.next().as_deref(), args.next()) {
        (Some("version"), None) => "version",
        (Some("banks"), None) => "banks",
        _ => return Err(format!("usage: {program} {{version|banks}}")),
    };

    let mut dongle = DonglePad::open().map_err(|error| error.to_string())?;
    match command {
        "version" => {
            let version = verify_version(&mut dongle)?;
            println!("{version}");
            Ok(())
        }
        "banks" => verify_banks(&mut dongle),
        _ => unreachable!(),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("ERROR: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_status() -> PadStatus {
        PadStatus {
            address: "via-s3-dongle".into(),
            name: Some("Cyberpad Val C6".into()),
            connected: true,
            paired: true,
            info: Some(EXPECTED_PAD_INFO.into()),
            transport: Some("dongle".into()),
            bluez_blocked: None,
            protocol_compatible: Some(true),
            slots_ready: Some(true),
            macro_ready: Some(true),
            selected_bank: Some(4),
        }
    }

    #[test]
    fn pair_gate_requires_every_runtime_signal() {
        assert_eq!(require_pair_ready(&ready_status(), "test").unwrap(), 4);

        let mut status = ready_status();
        status.connected = false;
        assert!(require_pair_ready(&status, "test").is_err());

        let mut status = ready_status();
        status.protocol_compatible = None;
        assert!(require_pair_ready(&status, "test").is_err());

        let mut status = ready_status();
        status.info = Some("Cyberdeck Pad Hybrid v0.2.0".into());
        assert!(require_pair_ready(&status, "test").is_err());

        let mut status = ready_status();
        status.slots_ready = Some(false);
        assert!(require_pair_ready(&status, "test").is_err());

        let mut status = ready_status();
        status.macro_ready = Some(false);
        assert!(require_pair_ready(&status, "test").is_err());

        let mut status = ready_status();
        status.selected_bank = Some(BANK_COUNT as u8);
        assert!(require_pair_ready(&status, "test").is_err());
    }
}
