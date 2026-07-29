//! Phase 1 inventory anchors + Phase 2 pointers.
//! Codec depth: `tests/slots_codec.rs`. Dispatch/composer: `mcc-desktop` modules.

use cyberdeck_ble::{HotkeySlot, PadSlots, DEVICE_NAME, MODE_HID, MODE_MACRO, SLOT_BYTES, SLOTS_BYTES};

#[test]
fn stub_compat_device_name_is_cyberdeck_pad() {
    // Compatibility identifier — do not rename without bonded-host review.
    assert_eq!(DEVICE_NAME, "Cyberdeck Pad");
}

#[test]
fn stub_slot_blob_length_matches_protocol() {
    let zeros = [0u8; SLOTS_BYTES];
    let slots = PadSlots::unpack(&zeros).expect("zero blob unpacks");
    let packed = slots.pack().expect("pack");
    assert_eq!(packed.len(), SLOTS_BYTES);
    assert_eq!(SLOT_BYTES, 27);
    assert!(std::mem::size_of::<HotkeySlot>() >= SLOT_BYTES);
}

#[test]
fn stub_mode_constants() {
    assert_eq!(MODE_HID, 0);
    assert_eq!(MODE_MACRO, 1);
}
