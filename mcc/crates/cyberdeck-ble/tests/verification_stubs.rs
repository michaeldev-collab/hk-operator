//! Phase 1 stubs: named integration points for future hardware-independent
//! and HITL verification. These tests document intent; they do not talk to
//! BlueZ. Phase 2 replaces ignores with real assertions where possible.

#[cfg(test)]
mod verification_stubs {
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
        // In-memory HotkeySlot may be larger than packed wire size due to alignment.
        assert!(std::mem::size_of::<HotkeySlot>() >= SLOT_BYTES);
    }

    #[test]
    fn stub_mode_constants() {
        assert_eq!(MODE_HID, 0);
        assert_eq!(MODE_MACRO, 1);
    }

    /// Placeholder for a future in-process binding→dispatch fixture (no shell).
    #[test]
    #[ignore = "Phase 2: dispatcher fixture not wired yet"]
    fn stub_dispatch_fixture_pending() {
        panic!("not implemented");
    }

    /// Placeholder for profile round-trip without touching ~/.config.
    #[test]
    #[ignore = "Phase 2: profile fixture harness not wired yet"]
    fn stub_profile_roundtrip_pending() {
        panic!("not implemented");
    }
}
