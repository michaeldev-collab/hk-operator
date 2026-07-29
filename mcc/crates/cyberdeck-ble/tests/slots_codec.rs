//! Phase 2: hardware-independent BLE slot codec + MacroEvent regressions.

use cyberdeck_ble::{
    HotkeySlot, MacroEvent, PadSlots, BleError, MODE_HID, MODE_MACRO, SLOT_BYTES, SLOT_COUNT,
    SLOTS_BYTES, PRESET_COUNT, ACTION_COUNT,
};

fn slot(mode: u8, r#mod: u8, key: u8, label: &str) -> HotkeySlot {
    HotkeySlot {
        mode,
        r#mod,
        key,
        label: label.into(),
    }
}

fn eighteen(template: HotkeySlot) -> PadSlots {
    PadSlots {
        slots: (0..SLOT_COUNT).map(|_| template.clone()).collect(),
    }
}

#[test]
fn hotkey_macro_and_hid_roundtrip() {
    let macro_slot = slot(MODE_MACRO, 0x01, 0x04, "Open");
    let packed = macro_slot.pack();
    assert_eq!(packed.len(), SLOT_BYTES);
    assert_eq!(packed[0], MODE_MACRO);
    let back = HotkeySlot::unpack(&packed).unwrap();
    assert_eq!(back.mode, MODE_MACRO);
    assert_eq!(back.r#mod, 0x01);
    assert_eq!(back.key, 0x04);
    assert_eq!(back.label, "Open");

    let hid = slot(MODE_HID, 0, 0x1e, "A");
    let back = HotkeySlot::unpack(&hid.pack()).unwrap();
    assert_eq!(back.mode, MODE_HID);
    assert_eq!(back.label, "A");
}

#[test]
fn pack_coerces_unknown_mode_to_hid() {
    let weird = slot(9, 0, 0, "x");
    assert_eq!(weird.pack()[0], MODE_HID);
}

#[test]
fn unpack_preserves_raw_mode_byte() {
    let mut buf = [0u8; SLOT_BYTES];
    buf[0] = 7;
    buf[3] = b'z';
    let s = HotkeySlot::unpack(&buf).unwrap();
    assert_eq!(s.mode, 7);
}

#[test]
fn label_truncates_to_23_bytes() {
    let long = "abcdefghijklmnopqrstuvwxyz"; // 26
    let packed = slot(MODE_MACRO, 0, 0, long).pack();
    let back = HotkeySlot::unpack(&packed).unwrap();
    assert_eq!(back.label.len(), 23);
    assert_eq!(back.label, &long[..23]);
}

#[test]
fn empty_label_roundtrips() {
    let back = HotkeySlot::unpack(&slot(MODE_HID, 0, 0, "").pack()).unwrap();
    assert_eq!(back.label, "");
}

#[test]
fn short_slot_buffer_errors() {
    let err = HotkeySlot::unpack(&[0u8; 10]).unwrap_err();
    assert!(matches!(err, BleError::BadSlotsLen { got: 10 }));
}

#[test]
fn pad_slots_wrong_count_and_short_blob() {
    let bad = PadSlots {
        slots: vec![slot(MODE_HID, 0, 0, ""); 3],
    };
    assert!(matches!(
        bad.pack().unwrap_err(),
        BleError::BadSlotsLen { .. }
    ));
    assert!(matches!(
        PadSlots::unpack(&[0u8; 100]).unwrap_err(),
        BleError::BadSlotsLen { got: 100 }
    ));
}

#[test]
fn pad_slots_full_roundtrip_length() {
    let slots = eighteen(slot(MODE_MACRO, 2, 3, "pad"));
    let packed = slots.pack().unwrap();
    assert_eq!(packed.len(), SLOTS_BYTES);
    let back = PadSlots::unpack(&packed).unwrap();
    assert_eq!(back.slots.len(), SLOT_COUNT);
    assert_eq!(back.slots[0].label, "pad");
    assert_eq!(back.slots[0].mode, MODE_MACRO);
}

#[test]
fn get_bounds_and_binding_key() {
    let slots = eighteen(slot(MODE_HID, 0, 0, ""));
    assert!(matches!(slots.get(PRESET_COUNT, 0), Err(BleError::BadIndex)));
    assert!(matches!(slots.get(0, ACTION_COUNT), Err(BleError::BadIndex)));
    assert!(slots.get(5, 2).is_ok());
    assert_eq!(PadSlots::binding_key(2, 1), "2-1");
}

#[test]
fn macro_event_from_bytes() {
    let ev = MacroEvent::from_bytes(&[3, 1]).unwrap();
    assert_eq!(ev.preset, 3);
    assert_eq!(ev.action, 1);
    assert!(MacroEvent::from_bytes(&[0, 0]).is_some());
    assert!(MacroEvent::from_bytes(&[5, 2]).is_some());
    assert!(MacroEvent::from_bytes(&[]).is_none());
    assert!(MacroEvent::from_bytes(&[1]).is_none());
    // Out-of-range indices dropped (P3-09)
    assert!(MacroEvent::from_bytes(&[6, 0]).is_none());
    assert!(MacroEvent::from_bytes(&[0, 3]).is_none());
    assert!(MacroEvent::from_bytes(&[255, 255]).is_none());
}

#[test]
fn zero_blob_unpack_repack() {
    let zeros = [0u8; SLOTS_BYTES];
    let slots = PadSlots::unpack(&zeros).unwrap();
    assert_eq!(slots.pack().unwrap().len(), SLOTS_BYTES);
}
