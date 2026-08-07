/** Shared onboard WS2812 connection/bank indicator. */
#pragma once

#include <Arduino.h>

#ifndef CONN_NEO_BRIGHT
#define CONN_NEO_BRIGHT 36
#endif
#ifndef CONN_NEO_FLASH_MS
#define CONN_NEO_FLASH_MS 400
#endif
#ifndef CONN_NEO_FLASH_FAST_MS
#define CONN_NEO_FLASH_FAST_MS 140
#endif
#ifndef CONN_NEO_PULSE_MS
#define CONN_NEO_PULSE_MS 1600
#endif

struct ConnNeoRgb {
  uint8_t r;
  uint8_t g;
  uint8_t b;
};

static constexpr ConnNeoRgb CONN_NEO_GREEN = {0, 255, 0};
static constexpr ConnNeoRgb CONN_NEO_BLUE  = {0, 0, 255};

static inline void connNeoSet(ConnNeoRgb rgb, uint8_t level = CONN_NEO_BRIGHT) {
#ifdef RGB_BUILTIN
  const uint8_t r = (uint16_t(rgb.r) * level) / 255;
  const uint8_t g = (uint16_t(rgb.g) * level) / 255;
  const uint8_t b = (uint16_t(rgb.b) * level) / 255;
  rgbLedWrite(RGB_BUILTIN, r, g, b);
#else
  (void)rgb;
  (void)level;
#endif
}

static inline void connNeoOff() { connNeoSet({0, 0, 0}, 0); }

/**
 * enabled=false -> off; disconnected/forceFlash -> flashing blue;
 * connected -> solid `linkedColour`, or a low-battery pulse when `pulse`.
 * The default colour keeps existing four-argument callers source-compatible.
 */
static inline void connNeoUpdate(bool connected, bool enabled, bool forceFlash,
                                 uint16_t flashMs,
                                 ConnNeoRgb linkedColour = CONN_NEO_GREEN,
                                 bool pulse = false) {
#ifdef RGB_BUILTIN
  static uint32_t lastToggleMs = 0;
  static uint32_t lastPulseDrawMs = 0;
  static bool flashOn = false;
  static bool wasFlashing = false;
  if (!enabled) {
    flashOn = false;
    wasFlashing = false;
    connNeoOff();
    return;
  }
  if (connected && !forceFlash) {
    wasFlashing = false;
    flashOn = true;
    if (!pulse) {
      connNeoSet(linkedColour);
      return;
    }
    const uint32_t now = millis();
    if (now - lastPulseDrawMs < 20) return;
    lastPulseDrawMs = now;
    const uint16_t half = CONN_NEO_PULSE_MS / 2;
    const uint16_t phase = now % CONN_NEO_PULSE_MS;
    const uint16_t ramp = phase <= half ? phase : CONN_NEO_PULSE_MS - phase;
    const uint8_t level = CONN_NEO_BRIGHT / 5 +
        (uint32_t(CONN_NEO_BRIGHT - CONN_NEO_BRIGHT / 5) * ramp) / half;
    connNeoSet(linkedColour, level);
    return;
  }
  uint16_t period = flashMs ? flashMs : CONN_NEO_FLASH_MS;
  uint32_t now = millis();
  if (!wasFlashing) {
    wasFlashing = true;
    flashOn = true;
    lastToggleMs = now;
    connNeoSet(CONN_NEO_BLUE);
    return;
  }
  if (now - lastToggleMs >= period) {
    lastToggleMs = now;
    flashOn = !flashOn;
    if (flashOn) connNeoSet(CONN_NEO_BLUE);
    else connNeoOff();
  }
#else
  (void)connected;
  (void)enabled;
  (void)forceFlash;
  (void)flashMs;
  (void)linkedColour;
  (void)pulse;
#endif
}
