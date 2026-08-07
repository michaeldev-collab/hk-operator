// Boot-time arbitration for GPIO12, which is both the green preset LED and the
// ESP32-C6's native USB D- line.
//
// The two functions are mutually exclusive at any given instant: driving the
// pin as an output tears down the USB differential pair (the host sees
// descriptor read failures and port power cycles, which reads like a boot loop
// and hides whatever is actually wrong), and leaving it to the USB peripheral
// means the LED just follows the idle bus level and sits lit with no software
// control.
//
// They are not, however, needed at the same time. The pad runs on battery
// almost always, and USB only matters when it is plugged in for flashing or
// debugging. So arbitrate once at boot:
//
//   USB host present -> USB keeps GPIO12; the NeoPixel carries preset colour
//   no USB host      -> GPIO12 becomes the green preset LED
//
// Detection is HWCDC::isPlugged(), which watches for USB start-of-frame
// packets, so it needs a moment of bus observation before it can answer -- it
// is sampled over a window rather than read once.
//
// This is deliberately a runtime decision, not a compile-time one. An earlier
// version gated the LED on CYBERPAD_USB_DIAGNOSTIC, which meant the debug build
// and the shipping build were different binaries; that is exactly the condition
// that let a boot-loop panic hide behind a USB fault for as long as it did.
//
// Caveat worth knowing: the decision is made once. If the pad boots on battery
// (GPIO12 = LED) and USB is plugged in later, the port will not enumerate until
// the board is reset, because the pin is no longer wired to the USB PHY.

#pragma once

#include <Arduino.h>
#include <HWCDC.h>

// ESP32-C6 USB Serial/JTAG pins.
static const int CPAD_USB_DM_PIN = 12;
static const int CPAD_USB_DP_PIN = 13;

// Held in a function-local static so this stays a header-only guard without
// tripping the one-definition rule. Defaults to "USB owns it" so that nothing
// can drive GPIO12 before arbitration has actually run.
inline bool &padLedUsbOwnsGreenRef() {
  static bool usbOwnsGreen = true;
  return usbOwnsGreen;
}

static inline bool padLedPinIsUsbReserved(int pin) {
  return padLedUsbOwnsGreenRef() &&
         (pin == CPAD_USB_DM_PIN || pin == CPAD_USB_DP_PIN);
}

// Arbitration is fail-safe toward USB and deliberately NOT a blocking sample in
// setup(). An earlier version sampled for 900 ms at boot and committed; because
// USB re-enumeration after a reset takes 1-2 s, it reliably concluded "no host"
// while plugged in, claimed GPIO12, and killed USB -- reintroducing the exact
// fault this guard exists to prevent. Never shorten the grace below the
// re-enumeration time, and never claim the pin on a single negative sample.
//
// Call padLedArbitrateTick() every loop. Until it decides, USB keeps the pin
// (the safe default), so a hang or a slow loop can never cost us USB. If a host
// is seen at any point during the grace window the decision locks to USB
// permanently. Only after the full window with no host ever seen does the green
// LED win. Returns true on the single tick where the LED was just claimed, so
// the caller can configure the pin and refresh the LEDs.
inline bool padLedArbitrateTick(uint32_t graceMs = 5000) {
  static bool decided = false;
  if (decided) return false;

  if (HWCDC::isPlugged()) {
    padLedUsbOwnsGreenRef() = true;  // already the default; lock it in
    decided = true;
    return false;
  }
  if (millis() >= graceMs) {
    padLedUsbOwnsGreenRef() = false;  // no host for the whole window: LED wins
    decided = true;
    return true;
  }
  return false;
}

static inline void padLedPinMode(int pin) {
  if (padLedPinIsUsbReserved(pin)) return;
  pinMode(pin, OUTPUT);
}

static inline void padLedDigitalWrite(int pin, bool on) {
  if (padLedPinIsUsbReserved(pin)) return;
  digitalWrite(pin, on ? HIGH : LOW);
}
