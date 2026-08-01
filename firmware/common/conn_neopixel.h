/**
 * Onboard WS2812 connection indicator (green).
 * Solid green = BLE link up; flashing = disconnected / mode cue.
 * When enabled=false (pad lights off), NeoPixel stays off.
 *
 * Uses Arduino-ESP32 rgbLedWrite + RGB_BUILTIN:
 *   ESP32-S3 Dev Module → GPIO48
 *   ESP32-C6 Dev Module → GPIO8
 */
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

static inline void connNeoSetGreen(uint8_t level) {
#ifdef RGB_BUILTIN
  rgbLedWrite(RGB_BUILTIN, 0, level, 0);
#else
  (void)level;
#endif
}

static inline void connNeoOff() { connNeoSetGreen(0); }

/**
 * connected → solid (unless forceFlash).
 * forceFlash → blink at flashMs (BlueZ fallback cue).
 * else → blink at flashMs while alone.
 * enabled=false → off.
 */
static inline void connNeoUpdate(bool connected, bool enabled, bool forceFlash,
                                 uint16_t flashMs) {
#ifdef RGB_BUILTIN
  static uint32_t lastToggleMs = 0;
  static bool flashOn = false;
  if (!enabled) {
    flashOn = false;
    connNeoOff();
    return;
  }
  if (connected && !forceFlash) {
    flashOn = true;
    connNeoSetGreen(CONN_NEO_BRIGHT);
    return;
  }
  uint16_t period = flashMs ? flashMs : CONN_NEO_FLASH_MS;
  uint32_t now = millis();
  if (now - lastToggleMs >= period) {
    lastToggleMs = now;
    flashOn = !flashOn;
    connNeoSetGreen(flashOn ? CONN_NEO_BRIGHT : 0);
  }
#else
  (void)connected;
  (void)enabled;
  (void)forceFlash;
  (void)flashMs;
#endif
}
