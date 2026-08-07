/**
 * c6-s3-dongle-validation.ino
 *
 * Validation-oriented copy of Cyberdeck Pad hybrid firmware.
 * Default (CYBERPAD_EXPERIMENTAL_S3_DONGLE=0): same hybrid BLE HID + GATT
 * behavior as ble-hid-hotkey-ble-config.
 *
 * Experimental (CYBERPAD_EXPERIMENTAL_S3_DONGLE=1):
 *   - S3 validation bridge preferred; direct BLE HID is the automatic fallback
 *   - Validation GATT (c0de1001) for keyboard-state notifies to S3
 *   - Hybrid Cyberdeck slots GATT (c0de0001) for MCC config via S3 proxy
 *   - B2/B4/B5 execute configured slots → validation KEYBOARD_REPORT
 *
 * Production firmware path remains:
 *   /run/media/stitch/data3/Operating/pi-iot/esp32/ble-hid-hotkeys/ble-hid-hotkey-ble-config/
 */

#include <Arduino.h>
#include <Preferences.h>

#ifndef CYBERPAD_EXPERIMENTAL_S3_DONGLE
#define CYBERPAD_EXPERIMENTAL_S3_DONGLE 0
#endif

#ifndef ENABLE_WIFI_FALLBACK
#define ENABLE_WIFI_FALLBACK 0
#endif

// Diagnostic builds keep the ESP32-C6 native USB/JTAG pins untouched and
// route Serial over hardware CDC. GPIO12 is both the prototype green LED and
// native USB D-, so that LED must remain disabled while this mode is active.
#ifndef CYBERPAD_USB_DIAGNOSTIC
#define CYBERPAD_USB_DIAGNOSTIC 0
#endif

#if CYBERPAD_USB_DIAGNOSTIC && !ARDUINO_USB_CDC_ON_BOOT
#error "CYBERPAD_USB_DIAGNOSTIC requires the CDCOnBoot=cdc board option"
#endif

#if CYBERPAD_EXPERIMENTAL_S3_DONGLE
#include <HijelHID_BLEKeyboard.h>
#include <NimBLEDevice.h>
#include <validation_protocol.h>
#include <conn_neopixel.h>
#else
#include <HijelHID_BLEKeyboard.h>
#include <NimBLEDevice.h>
#if ENABLE_WIFI_FALLBACK
#include <WiFi.h>
#include <WebServer.h>
#endif
#endif

// ─── Pins (existing Cyberpad prototype) ─────────────────────────────────────
const int BUTTON1 = 2;
const int BUTTON2 = 3;
const int BUTTON3 = 4;
const int BUTTON4 = 6;
const int BUTTON5 = 5;

const int LED_GREEN = 12;
const int LED_RED   = 21;
const int LED_BLUE  = 15;

// LED_GREEN is GPIO12, which is also the C6's native USB D- line. Every pad LED
// write in BOTH firmware branches goes through these guards so diagnostic builds
// keep USB alive. No-op unless CYBERPAD_USB_DIAGNOSTIC=1.
#include "pad_led_guard.h"

static const int PRESET_COUNT = 6;
static const int ACTION_COUNT = 3;

int  currentPreset = 1;
bool lightsEnabled = true;

bool lastButton1 = HIGH;
bool lastButton2 = HIGH;
bool lastButton3 = HIGH;
bool lastButton4 = HIGH;
bool lastButton5 = HIGH;

// ═══════════════════════════════════════════════════════════════════════════
#if CYBERPAD_EXPERIMENTAL_S3_DONGLE
// ═══════════════════════════════════════════════════════════════════════════

static const char *CYBERDECK_SERVICE_UUID   = "c0de0001-3d17-4a00-8000-00805f9b34fb";
static const char *CYBERDECK_SLOTS_UUID     = "c0de0002-3d17-4a00-8000-00805f9b34fb";
static const char *CYBERDECK_MACRO_EVT_UUID = "c0de0003-3d17-4a00-8000-00805f9b34fb";
static const char *CYBERDECK_INFO_UUID      = "c0de0004-3d17-4a00-8000-00805f9b34fb";
static const char *CYBERDECK_BANK_SEL_UUID  = "c0de0005-3d17-4a00-8000-00805f9b34fb";
static const char *BATTERY_SERVICE_UUID     = "180f";
static const char *BATTERY_LEVEL_UUID       = "2a19";
static const char *FW_INFO = "Cyberdeck Pad Hybrid v0.3.1";
static const char *ADV_NAME = "Cyberpad Val C6";

static const uint8_t MODE_HID   = 0;
static const uint8_t MODE_MACRO = 1;
static const uint8_t BANK_COUNT = 5;
static const uint8_t NVS_SCHEMA_VERSION = 3;

static const uint8_t BATTERY_PIN = 1;
static const uint16_t BAT_FULL_MV = 4200;
static const uint16_t BAT_EMPTY_MV = 3400;
static const float BAT_DIVIDER = 2.0f;
static const uint8_t BAT_LOW_PCT = 15;
static const uint8_t BAT_OVERSAMPLE_READS = 16;
static const uint32_t BAT_SAMPLE_INTERVAL_MS = 30000;

static const ConnNeoRgb BANK_COLOURS[BANK_COUNT] = {
    {0, 255, 0},    // desktop: green
    {255, 96, 0},   // dev: amber
    {255, 0, 160},  // browser: magenta
    {255, 0, 0},    // misc: red
    {255, 255, 255} // overflow: white
};

struct Hotkey {
  uint8_t mode;
  uint8_t mod;
  uint8_t key;
  char    label[24];
};

static const size_t SLOT_BYTES = sizeof(Hotkey);
static const size_t BANK_SLOTS_BYTES = SLOT_BYTES * PRESET_COUNT * ACTION_COUNT;
static const size_t SLOTS_PAGE_BYTES = 1 + BANK_SLOTS_BYTES;
static_assert(SLOT_BYTES == 27, "v0.3 preserves the 27-byte slot record");
static_assert(BANK_SLOTS_BYTES == 486, "v0.3 bank page payload must be 486 bytes");
static_assert(SLOTS_PAGE_BYTES == 487, "v0.3 Slots characteristic must be 487 bytes");

HijelHID_BLEKeyboard keyboard(ADV_NAME, "Stitch", 100);
Preferences prefs;
Hotkey hotkeys[BANK_COUNT][PRESET_COUNT][ACTION_COUNT];
portMUX_TYPE gHotkeysMux = portMUX_INITIALIZER_UNLOCKED;

NimBLECharacteristic *pValNotify    = nullptr;
NimBLECharacteristic *pSlotsChar    = nullptr;
NimBLECharacteristic *pMacroEvtChar = nullptr;
NimBLECharacteristic *pBankSelChar  = nullptr;
NimBLECharacteristic *pBatteryChar  = nullptr;
bool gValSubscribed = false;
volatile bool gValSubscriptionPending = false;
volatile bool gValSubscriptionRequested = false;
portMUX_TYPE gValSubscriptionMux = portMUX_INITIALIZER_UNLOCKED;
bool gattMacroSubscribed = false;
uint8_t gCurrentBank = 0;
uint16_t gValSeq = 1;
uint32_t gLastHeartbeatMs = 0;
uint32_t gLastAdvKickMs = 0;
int gHeldAction = -1; // 0..2 while HID key held via B2/B4/B5
bool gBluezFallback = false; // long-press B1: BLE HID to host instead of S3 bridge
uint32_t gB1DownAtMs = 0;
bool gB1LongHandled = false;
uint32_t gB3DownAtMs = 0;
bool gB3LongHandled = false;
uint8_t gBatteryPct = 100;
uint16_t gBatteryFilteredMv = 0;
uint32_t gLastBatterySampleMs = 0;
int8_t gPendingBatteryDirection = 0;
uint8_t gPendingBatterySamples = 0;
String gLine;

#ifndef BLUEZ_FALLBACK_HOLD_MS
#define BLUEZ_FALLBACK_HOLD_MS 1200
#endif

/* 75% duty — 25% dimmer than full on, matching CONN_NEO_BRIGHT. */
#ifndef PAD_LED_LEVEL
#define PAD_LED_LEVEL 191
#endif

static inline void padLedWrite(int pin, bool on) {
  if (padLedPinIsUsbReserved(pin)) return;
  analogWrite(pin, on ? PAD_LED_LEVEL : 0);
}

#if CYBERPAD_USB_DIAGNOSTIC
static const char *diagResetReasonName(esp_reset_reason_t reason) {
  switch (reason) {
    case ESP_RST_UNKNOWN: return "unknown";
    case ESP_RST_POWERON: return "power-on/external-reset";
    case ESP_RST_EXT: return "external-pin";
    case ESP_RST_SW: return "software-restart";
    case ESP_RST_PANIC: return "panic";
    case ESP_RST_INT_WDT: return "interrupt-watchdog";
    case ESP_RST_TASK_WDT: return "task-watchdog";
    case ESP_RST_WDT: return "other-watchdog";
    case ESP_RST_DEEPSLEEP: return "deep-sleep";
    case ESP_RST_BROWNOUT: return "brownout";
    case ESP_RST_SDIO: return "sdio";
    default: return "other";
  }
}
#endif


void setDefaults() {
  memset(hotkeys, 0, sizeof(hotkeys));
  hotkeys[0][0][0] = { MODE_HID, 0,                              KEY_RETURN, "Enter" };
  hotkeys[0][0][1] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_S,      "Ctrl+S (Save)" };
  hotkeys[0][0][2] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_V,      "Ctrl+V (Paste)" };
  hotkeys[0][1][0] = { MODE_HID, 0,                              KEY_RETURN, "Enter" };
  hotkeys[0][1][1] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LSHIFT, KEY_C,      "Ctrl+Shift+C" };
  hotkeys[0][1][2] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LSHIFT, KEY_V,      "Ctrl+Shift+V" };
  hotkeys[0][2][0] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LALT,   KEY_1,      "Ctrl+Alt+1" };
  hotkeys[0][2][1] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LALT,   KEY_2,      "Ctrl+Alt+2" };
  hotkeys[0][2][2] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LALT,   KEY_3,      "Ctrl+Alt+3" };
  hotkeys[0][3][0] = { MODE_HID, 0,                              KEY_RETURN, "Enter" };
  hotkeys[0][3][1] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_C,      "Ctrl+C" };
  hotkeys[0][3][2] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_X,      "Ctrl+X" };
  hotkeys[0][4][0] = { MODE_HID, 0,                              KEY_TAB,    "Tab" };
  hotkeys[0][4][1] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_Z,      "Ctrl+Z" };
  hotkeys[0][4][2] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LSHIFT, KEY_Z,      "Ctrl+Shift+Z" };
  hotkeys[0][5][0] = { MODE_HID, KEY_MOD_LALT,                   KEY_TAB,    "Alt+Tab" };
  hotkeys[0][5][1] = { MODE_HID, 0,                              KEY_ESCAPE, "Esc" };
  hotkeys[0][5][2] = { MODE_HID, KEY_MOD_LGUI,                   KEY_L,      "Gui+L" };
}

void packSlots(uint8_t bank, uint8_t *out) {
  out[0] = bank;
  size_t i = 1;
  portENTER_CRITICAL(&gHotkeysMux);
  for (int p = 0; p < PRESET_COUNT; p++) {
    for (int a = 0; a < ACTION_COUNT; a++) {
      memcpy(out + i, &hotkeys[bank][p][a], SLOT_BYTES);
      i += SLOT_BYTES;
    }
  }
  portEXIT_CRITICAL(&gHotkeysMux);
}

bool validUtf8Label(const char *label) {
  const uint8_t *s = reinterpret_cast<const uint8_t *>(label);
  size_t len = 0;
  while (len < 24 && s[len] != 0) len++;
  if (len == 24) return false;
  for (size_t i = len + 1; i < 24; i++) {
    if (s[i] != 0) return false; // NUL-padded, not just NUL-terminated.
  }
  for (size_t i = 0; i < len;) {
    const uint8_t c = s[i++];
    if (c <= 0x7f) continue;
    if (c >= 0xc2 && c <= 0xdf) {
      if (i >= len || (s[i++] & 0xc0) != 0x80) return false;
      continue;
    }
    if (c >= 0xe0 && c <= 0xef) {
      if (i + 1 >= len) return false;
      const uint8_t c1 = s[i++];
      const uint8_t c2 = s[i++];
      if ((c2 & 0xc0) != 0x80 ||
          (c == 0xe0 ? c1 < 0xa0 || c1 > 0xbf
                     : c == 0xed ? c1 < 0x80 || c1 > 0x9f
                                : (c1 & 0xc0) != 0x80)) return false;
      continue;
    }
    if (c >= 0xf0 && c <= 0xf4) {
      if (i + 2 >= len) return false;
      const uint8_t c1 = s[i++];
      const uint8_t c2 = s[i++];
      const uint8_t c3 = s[i++];
      if ((c2 & 0xc0) != 0x80 || (c3 & 0xc0) != 0x80 ||
          (c == 0xf0 ? c1 < 0x90 || c1 > 0xbf
                     : c == 0xf4 ? c1 < 0x80 || c1 > 0x8f
                                : (c1 & 0xc0) != 0x80)) return false;
      continue;
    }
    return false;
  }
  return true;
}

bool unpackSlots(const uint8_t *in, size_t len,
                 Hotkey out[PRESET_COUNT][ACTION_COUNT]) {
  if (len != BANK_SLOTS_BYTES) return false;
  size_t i = 0;
  for (int p = 0; p < PRESET_COUNT; p++) {
    for (int a = 0; a < ACTION_COUNT; a++) {
      Hotkey h;
      memcpy(&h, in + i, SLOT_BYTES);
      i += SLOT_BYTES;
      if (h.mode > MODE_MACRO || !validUtf8Label(h.label)) return false;
      out[p][a] = h;
    }
  }
  return true;
}

bool saveBankData(uint8_t bank, const Hotkey *data) {
  if (bank >= BANK_COUNT) return false;
  char key[8];
  snprintf(key, sizeof(key), "slots%u", (unsigned)bank);
  const size_t written = prefs.putBytes(key, data, BANK_SLOTS_BYTES);
  if (written != BANK_SLOTS_BYTES) {
    Serial.printf("[nvs] ERROR bank=%u wrote=%u want=%u\n", (unsigned)bank,
                  (unsigned)written, (unsigned)BANK_SLOTS_BYTES);
    return false;
  }
  return true;
}

bool saveBank(uint8_t bank) {
  return bank < BANK_COUNT && saveBankData(bank, &hotkeys[bank][0][0]);
}

bool saveConfig() {
  bool ok = true;
  for (uint8_t bank = 0; bank < BANK_COUNT; bank++) ok = saveBank(bank) && ok;
  if (!ok) return false;
  const size_t marked = prefs.putUChar("schema", NVS_SCHEMA_VERSION);
  if (marked != sizeof(uint8_t)) {
    Serial.println("[nvs] ERROR could not commit v0.3 schema marker");
    return false;
  }
  return true;
}

bool validateStoredBank(uint8_t bank) {
  if (bank >= BANK_COUNT) return false;
  Hotkey validated[PRESET_COUNT][ACTION_COUNT];
  if (!unpackSlots(reinterpret_cast<const uint8_t *>(hotkeys[bank]),
                   BANK_SLOTS_BYTES, validated)) return false;
  memcpy(hotkeys[bank], validated, BANK_SLOTS_BYTES);
  return true;
}

void loadConfig() {
  if (!prefs.begin("hotkeys3", false)) {
    Serial.println("[nvs] ERROR open failed; using volatile defaults");
    setDefaults();
    gCurrentBank = 0;
    return;
  }
  bool loadedBanks = true;
  for (uint8_t bank = 0; bank < BANK_COUNT; bank++) {
    char key[8];
    snprintf(key, sizeof(key), "slots%u", (unsigned)bank);
    if (prefs.getBytes(key, hotkeys[bank], BANK_SLOTS_BYTES) != BANK_SLOTS_BYTES ||
        !validateStoredBank(bank)) {
      loadedBanks = false;
      break;
    }
  }
  if (loadedBanks) {
    if (prefs.getUChar("schema", 0) != NVS_SCHEMA_VERSION &&
        prefs.putUChar("schema", NVS_SCHEMA_VERSION) != sizeof(uint8_t)) {
      Serial.println("[nvs] WARNING loaded banks but schema marker write failed");
    }
    Serial.println("[nvs] loaded five bank pages");
  } else if (prefs.getUChar("schema", 0) == NVS_SCHEMA_VERSION) {
    // A committed v0.3 store must never fall back to stale legacy data. Keep
    // every valid page, replace only damaged/missing pages with defaults, and
    // rewrite the complete set plus marker as one recoverable generation.
    setDefaults();
    uint8_t recovered = 0;
    for (uint8_t bank = 0; bank < BANK_COUNT; bank++) {
      char key[8];
      snprintf(key, sizeof(key), "slots%u", (unsigned)bank);
      Hotkey raw[PRESET_COUNT][ACTION_COUNT];
      Hotkey validated[PRESET_COUNT][ACTION_COUNT];
      if (prefs.getBytes(key, raw, BANK_SLOTS_BYTES) == BANK_SLOTS_BYTES &&
          unpackSlots(reinterpret_cast<const uint8_t *>(raw), BANK_SLOTS_BYTES,
                      validated)) {
        memcpy(hotkeys[bank], validated, BANK_SLOTS_BYTES);
        recovered++;
      }
    }
    Serial.printf("[nvs] repaired committed v0.3 store (%u/%u pages recovered): %s\n",
                  (unsigned)recovered, (unsigned)BANK_COUNT,
                  saveConfig() ? "OK" : "FAILED");
  } else {
    memset(hotkeys, 0, sizeof(hotkeys));
    const size_t monolithic = prefs.getBytes("slots5", hotkeys, sizeof(hotkeys));
    bool monolithicValid = monolithic == sizeof(hotkeys);
    for (uint8_t bank = 0; bank < BANK_COUNT && monolithicValid; bank++) {
      monolithicValid = validateStoredBank(bank);
    }
    if (monolithicValid) {
      Serial.printf("[nvs] migrated monolithic v0.3 slots: %s\n",
                    saveConfig() ? "OK" : "FAILED");
    } else {
      memset(hotkeys, 0, sizeof(hotkeys));
      const size_t legacy = prefs.getBytes("slots", hotkeys[0], BANK_SLOTS_BYTES);
      if (legacy == BANK_SLOTS_BYTES && validateStoredBank(0)) {
        Serial.printf("[nvs] migrated v0.2 slots into bank 0: %s\n",
                      saveConfig() ? "OK" : "FAILED");
      } else {
        setDefaults();
        Serial.printf("[nvs] initialized v0.3 defaults: %s\n",
                      saveConfig() ? "OK" : "FAILED");
      }
    }
  }
  // Bank selection is transient UI state; avoiding an NVS write on every B3
  // press materially reduces wear without risking any slot data.
  gCurrentBank = 0;
}

void setCurrentBank(uint8_t bank, bool notify) {
  if (bank >= BANK_COUNT) return;
  const bool changed = bank != gCurrentBank;
  gCurrentBank = bank;
  if (changed) {
    Serial.printf("[bank] selected %u\n", (unsigned)gCurrentBank);
  }
  if (pBankSelChar) {
    pBankSelChar->setValue(&gCurrentBank, 1);
    if (notify && changed) pBankSelChar->notify();
  }
  if (pSlotsChar) {
    uint8_t buf[SLOTS_PAGE_BYTES];
    packSlots(gCurrentBank, buf);
    pSlotsChar->setValue(buf, sizeof(buf));
  }
}

void refreshSlotsCharacteristic() {
  if (!pSlotsChar) return;
  uint8_t buf[SLOTS_PAGE_BYTES];
  packSlots(gCurrentBank, buf);
  pSlotsChar->setValue(buf, sizeof(buf));
}

uint8_t batteryPctFromMv(uint16_t mv) {
  struct Point { uint16_t mv; uint8_t pct; };
  static const Point curve[] = {
      {3400, 0}, {3680, 10}, {3740, 20}, {3770, 30}, {3790, 40},
      {3820, 50}, {3870, 60}, {3920, 70}, {3980, 80}, {4060, 90},
      {4200, 100},
  };
  if (mv <= BAT_EMPTY_MV) return 0;
  if (mv >= BAT_FULL_MV) return 100;
  for (size_t i = 1; i < sizeof(curve) / sizeof(curve[0]); i++) {
    if (mv <= curve[i].mv) {
      const uint16_t spanMv = curve[i].mv - curve[i - 1].mv;
      const uint8_t spanPct = curve[i].pct - curve[i - 1].pct;
      return curve[i - 1].pct +
          (uint32_t(mv - curve[i - 1].mv) * spanPct) / spanMv;
    }
  }
  return 100;
}

uint16_t readBatteryMv() {
  uint32_t pinMvSum = 0;
  for (uint8_t i = 0; i < BAT_OVERSAMPLE_READS; i++) {
    pinMvSum += analogReadMilliVolts(BATTERY_PIN);
    delay(2);
  }
  const uint16_t pinMv = pinMvSum / BAT_OVERSAMPLE_READS;
  const uint32_t cellMv = uint32_t(float(pinMv) * BAT_DIVIDER + 0.5f);
  return cellMv > 65535U ? 65535U : uint16_t(cellMv);
}

void publishBattery(uint8_t pct) {
  gBatteryPct = pct > 100 ? 100 : pct;
  if (pBatteryChar) {
    pBatteryChar->setValue(&gBatteryPct, 1);
    pBatteryChar->notify();
    // HijelHID uses the same safety gap after BAS notifications. Without it,
    // an immediately following validation/HID report can lose the shared ACL
    // buffer and strand a key-down state on the host.
    delay(30);
  }
  Serial.printf("[battery] %u%% filtered=%umV\n",
                (unsigned)gBatteryPct, (unsigned)gBatteryFilteredMv);
}

void sampleBattery(bool immediate) {
  const uint32_t now = millis();
  if (!immediate && now - gLastBatterySampleMs < BAT_SAMPLE_INTERVAL_MS) return;
  gLastBatterySampleMs = now;
  const uint16_t measuredMv = readBatteryMv();
  if (gBatteryFilteredMv == 0) gBatteryFilteredMv = measuredMv;
  else gBatteryFilteredMv = (uint32_t(gBatteryFilteredMv) * 3U + measuredMv) / 4U;
  const uint8_t candidate = batteryPctFromMv(gBatteryFilteredMv);
  if (immediate) {
    gPendingBatteryDirection = 0;
    gPendingBatterySamples = 0;
    publishBattery(candidate);
    return;
  }
  const int delta = int(candidate) - int(gBatteryPct);
  if (delta == 0) {
    gPendingBatteryDirection = 0;
    gPendingBatterySamples = 0;
    return;
  }
  const int8_t direction = delta > 0 ? 1 : -1;
  if (gPendingBatteryDirection == direction) gPendingBatterySamples++;
  else {
    gPendingBatteryDirection = direction;
    gPendingBatterySamples = 1;
  }
  // Require two consecutive samples moving in the same direction. Requiring
  // the exact same percentage would stall a genuine gradual discharge.
  if (gPendingBatterySamples >= 2 && abs(delta) >= 1) {
    publishBattery(candidate);
    gPendingBatteryDirection = 0;
    gPendingBatterySamples = 0;
  }
}

void updatePresetLeds() {
  if (!lightsEnabled) {
    padLedWrite(LED_RED, false);
    padLedWrite(LED_GREEN, false);
    padLedWrite(LED_BLUE, false);
    return;
  }
  bool r = false, g = false, b = false;
  switch (currentPreset) {
    case 1: r = true; break;
    case 2: g = true; break;
    case 3: b = true; break;
    case 4: r = true; g = true; break;
    case 5: g = true; b = true; break;
    case 6: r = true; b = true; break;
    default: break;
  }
  padLedWrite(LED_RED,   r);
  padLedWrite(LED_GREEN, g);
  padLedWrite(LED_BLUE,  b);
}

void notifyPacket(const cpad_val_packet_t &pkt) {
  if (!pValNotify) return;
  pValNotify->setValue((uint8_t *)&pkt, CPAD_VAL_PACKET_SIZE);
  pValNotify->notify();
}

void sendKeyboardState(uint8_t modifiers, const uint8_t keys[6]) {
  cpad_val_packet_t pkt;
  cpad_val_encode(&pkt, CPAD_VAL_MSG_KEYBOARD_REPORT, gValSeq++, modifiers, keys);
  notifyPacket(pkt);
  Serial.printf("[val] KEYBOARD_REPORT seq=%u mod=0x%02x key0=0x%02x\n",
                (unsigned)pkt.seq, pkt.modifiers, pkt.keys[0]);
}

void sendReleaseAll() {
  cpad_val_packet_t pkt;
  cpad_val_encode(&pkt, CPAD_VAL_MSG_RELEASE_ALL, gValSeq++, 0, nullptr);
  notifyPacket(pkt);
  Serial.println("[val] RELEASE_ALL");
}

void sendHello() {
  cpad_val_packet_t pkt;
  cpad_val_encode(&pkt, CPAD_VAL_MSG_HELLO, gValSeq++, 0, nullptr);
  notifyPacket(pkt);
  Serial.println("[val] HELLO");
}

void sendHeartbeat() {
  cpad_val_packet_t pkt;
  cpad_val_encode(&pkt, CPAD_VAL_MSG_HEARTBEAT, gValSeq++, 0, nullptr);
  notifyPacket(pkt);
}

void sendLightsState() {
  cpad_val_packet_t pkt;
  cpad_val_encode(&pkt, CPAD_VAL_MSG_LIGHTS, gValSeq++, lightsEnabled ? 1 : 0, nullptr);
  notifyPacket(pkt);
  Serial.printf("[val] LIGHTS %s\n", lightsEnabled ? "on" : "off");
}

void notifyMacroEvent(uint8_t bank, uint8_t presetIdx, uint8_t actionIdx) {
  if (!pMacroEvtChar) return;
  uint8_t payload[3] = {bank, presetIdx, actionIdx};
  pMacroEvtChar->setValue(payload, sizeof(payload));
  pMacroEvtChar->notify();
  (void)gattMacroSubscribed;
}

void pressAction(int actionIdx) {
  if (actionIdx < 0 || actionIdx >= ACTION_COUNT) return;
  const uint8_t bank = gCurrentBank;
  const uint8_t presetIdx = uint8_t(currentPreset - 1);
  Hotkey h;
  portENTER_CRITICAL(&gHotkeysMux);
  h = hotkeys[bank][presetIdx][actionIdx];
  portEXIT_CRITICAL(&gHotkeysMux);
  if (h.mode == MODE_MACRO) {
    notifyMacroEvent(bank, presetIdx, uint8_t(actionIdx));
    return;
  }
  if (h.key == KEY_NONE) return;

  // No validation subscriber means the S3 is not there to relay, so direct HID
  // is the only path that can reach a host. Requiring isPaired() here dropped
  // presses whenever the library's auth flag lagged the encrypted BlueZ link.
  const bool useDirectHid = gBluezFallback || !gValSubscribed;
  if (useDirectHid) {
    if (!keyboard.isConnected() && !keyboard.isPaired()) {
      Serial.println("[hid] no HID host connected — press dropped");
      return;
    }
    keyboard.tap(h.key, h.mod);
    Serial.printf("[hid] tap fallback key=0x%02x mod=0x%02x\n", h.key, h.mod);
    return;
  }

  uint8_t keys[6] = {h.key, 0, 0, 0, 0, 0};
  sendKeyboardState(h.mod, keys);
  gHeldAction = actionIdx;
}

void releaseHeldAction() {
  if (gHeldAction < 0) return;
  if (gBluezFallback) {
    gHeldAction = -1;
    return;
  }
  uint8_t empty[6] = {0};
  sendKeyboardState(0, empty);
  gHeldAction = -1;
}

void setBluezFallback(bool on) {
  if (gBluezFallback == on) return;
  releaseHeldAction();
  gBluezFallback = on;
  if (gBluezFallback) {
    Serial.println("[mode] BlueZ HID fallback ON — long-press B1 again for S3 dongle mode");
    Serial.println("[mode] Unblock/connect the pad in host BlueZ if HID does not appear");
    // Re-advertise so a host central can attach for HID.
    startValidationAdvertising();
  } else {
    Serial.println("[mode] S3 dongle mode ON — validation GATT path");
    startValidationAdvertising();
  }
}

class ValNotifyCallbacks : public NimBLECharacteristicCallbacks {
  void onSubscribe(NimBLECharacteristic * /*pChar*/, NimBLEConnInfo & /*connInfo*/,
                   uint16_t subValue) override {
    // NimBLE invokes this on its host task. Only publish the requested state;
    // the Arduino loop owns notification sequencing and held-key transitions.
    // This prevents subscribe/drop callbacks from interleaving setValue() /
    // notify() with a button report and losing its matching release.
    portENTER_CRITICAL(&gValSubscriptionMux);
    gValSubscriptionRequested = (subValue != 0);
    gValSubscriptionPending = true;
    portEXIT_CRITICAL(&gValSubscriptionMux);
  }
};

void processValidationSubscription() {
  bool pending = false;
  bool requested = false;
  portENTER_CRITICAL(&gValSubscriptionMux);
  pending = gValSubscriptionPending;
  if (pending) {
    requested = gValSubscriptionRequested;
    gValSubscriptionPending = false;
  }
  portEXIT_CRITICAL(&gValSubscriptionMux);
  if (!pending) return;

  gValSubscribed = requested;
  Serial.printf("[val] subscribe=%u\n", requested ? 1U : 0U);
  if (requested) {
    sendHello();
    sendLightsState();
    gLastHeartbeatMs = millis();
  } else {
    releaseHeldAction();
  }
}

class SlotsCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic *pChar, NimBLEConnInfo &connInfo) override {
    (void)connInfo;
    std::string val = pChar->getValue();
    const uint8_t selectedBank = gCurrentBank;
    if (val.size() != SLOTS_PAGE_BYTES ||
        uint8_t(val[0]) != selectedBank) {
      refreshSlotsCharacteristic();
      Serial.printf("[slots] reject len=%u payload_bank=%d selected=%u\n",
                    (unsigned)val.size(), val.empty() ? -1 : uint8_t(val[0]),
                    (unsigned)selectedBank);
      return;
    }
    Hotkey candidate[PRESET_COUNT][ACTION_COUNT];
    if (!unpackSlots(reinterpret_cast<const uint8_t *>(val.data()) + 1,
                     BANK_SLOTS_BYTES, candidate)) {
      refreshSlotsCharacteristic();
      Serial.printf("[slots] reject bank=%u: invalid slot record\n",
                    (unsigned)selectedBank);
      return;
    }
    // Persist the validated page before publishing it to live action state.
    // Every step uses the captured bank, so a concurrent B3/BankSel change can
    // only make the write stale and visible on a later read, never redirect it.
    if (!saveBankData(selectedBank, &candidate[0][0])) {
      refreshSlotsCharacteristic();
      Serial.printf("[slots] reject bank=%u: persistence failed\n",
                    (unsigned)selectedBank);
      return;
    }
    portENTER_CRITICAL(&gHotkeysMux);
    memcpy(hotkeys[selectedBank], candidate, BANK_SLOTS_BYTES);
    portEXIT_CRITICAL(&gHotkeysMux);
    refreshSlotsCharacteristic();
    Serial.printf("[slots] wrote bank=%u from GATT\n", (unsigned)selectedBank);
  }
  // Deliberately NO onRead refresh. The 487-byte page cannot fit one ATT chunk
  // at the negotiated MTU, so clients fetch it as Read + Read Blob
  // continuations -- and onRead fires for EVERY one of those ops. Calling
  // setValue() mid-long-read replaces the value buffer between chunks, so each
  // continuation served stale heap (old label fragments) from byte MTU-1
  // onward: deterministic slot-10 corruption at offset 271, garbage varying
  // between reads. The value is already refreshed at every mutation site
  // (setCurrentBank, every onWrite path, GATT setup), so a read-time refresh
  // adds nothing but the corruption.
};

class BankSelCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic *pChar, NimBLEConnInfo &connInfo) override {
    (void)connInfo;
    std::string val = pChar->getValue();
    if (val.size() != 1 || uint8_t(val[0]) >= BANK_COUNT) {
      pChar->setValue(&gCurrentBank, 1);
      Serial.println("[bank] rejected out-of-range write");
      return;
    }
    setCurrentBank(uint8_t(val[0]), true);
  }
  void onRead(NimBLECharacteristic *pChar, NimBLEConnInfo & /*connInfo*/) override {
    pChar->setValue(&gCurrentBank, 1);
  }
};

class MacroEvtCallbacks : public NimBLECharacteristicCallbacks {
  void onSubscribe(NimBLECharacteristic * /*pChar*/, NimBLEConnInfo & /*connInfo*/,
                   uint16_t subValue) override {
    gattMacroSubscribed = (subValue != 0);
  }
};

ValNotifyCallbacks valCb;
SlotsCallbacks slotsCb;
BankSelCallbacks bankSelCb;
MacroEvtCallbacks macroEvtCb;

static void ledStage(bool r, bool g, bool b) {
  padLedWrite(LED_RED, r);
  padLedWrite(LED_GREEN, g);
  padLedWrite(LED_BLUE, b);
}

static void startValidationAdvertising() {
  NimBLEDevice::stopAdvertising();
  NimBLEAdvertising *adv = NimBLEDevice::getAdvertising();
  adv->addServiceUUID(CPAD_VAL_SERVICE_UUID);
  adv->addServiceUUID(CYBERDECK_SERVICE_UUID);
  adv->enableScanResponse(true);
  adv->setMinInterval(0x20);
  adv->setMaxInterval(0x40);
  NimBLEAdvertisementData scanResponse;
  scanResponse.setName(ADV_NAME);
  adv->setScanResponseData(scanResponse);
  bool ok = NimBLEDevice::startAdvertising();
  Serial.printf("[val] startAdvertising => %s\n", ok ? "OK" : "FAIL");
  if (ok) {
    ledStage(false, false, true);
    delay(150);
    updatePresetLeds();
  } else {
    ledStage(true, false, true);
  }
}

void setupHybridGatt(NimBLEServer *server) {
  NimBLEDevice::setMTU(517);
  NimBLEService *svc = server->createService(CYBERDECK_SERVICE_UUID);
  pSlotsChar = svc->createCharacteristic(
      CYBERDECK_SLOTS_UUID,
      NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::WRITE);
  pSlotsChar->setCallbacks(&slotsCb);
  refreshSlotsCharacteristic();
  pBankSelChar = svc->createCharacteristic(
      CYBERDECK_BANK_SEL_UUID,
      NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::WRITE | NIMBLE_PROPERTY::NOTIFY);
  pBankSelChar->setCallbacks(&bankSelCb);
  pBankSelChar->setValue(&gCurrentBank, 1);
  pMacroEvtChar = svc->createCharacteristic(
      CYBERDECK_MACRO_EVT_UUID,
      NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::NOTIFY);
  pMacroEvtChar->setCallbacks(&macroEvtCb);
  uint8_t zero[3] = {0, 0, 0};
  pMacroEvtChar->setValue(zero, sizeof(zero));
  NimBLECharacteristic *pInfo =
      svc->createCharacteristic(CYBERDECK_INFO_UUID, NIMBLE_PROPERTY::READ);
  pInfo->setValue(FW_INFO);
  svc->start();
  Serial.println("[slots] hybrid Cyberdeck GATT ready");
}

void setupValidationGatt() {
  ledStage(true, false, false);
  Serial.println("[val] HijelHID begin (stack bring-up only)...");
  keyboard.begin();
  ledStage(false, true, false);

  NimBLEServer *server = NimBLEDevice::getServer();
  if (!server) {
    Serial.println("[val] ERROR: no NimBLE server after keyboard.begin()");
    ledStage(true, false, true);
    return;
  }

  NimBLEService *batteryService = server->getServiceByUUID(BATTERY_SERVICE_UUID);
  if (batteryService) {
    pBatteryChar = batteryService->getCharacteristic(BATTERY_LEVEL_UUID);
    if (pBatteryChar) pBatteryChar->setValue(&gBatteryPct, 1);
  }
  Serial.printf("[battery] BAS %s\n", pBatteryChar ? "ready" : "missing");

  NimBLEService *svc = server->createService(CPAD_VAL_SERVICE_UUID);
  pValNotify = svc->createCharacteristic(
      CPAD_VAL_NOTIFY_UUID,
      NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::NOTIFY);
  pValNotify->setCallbacks(&valCb);
  cpad_val_packet_t zero;
  cpad_val_encode(&zero, CPAD_VAL_MSG_RELEASE_ALL, 0, 0, nullptr);
  pValNotify->setValue((uint8_t *)&zero, CPAD_VAL_PACKET_SIZE);
  svc->start();

  setupHybridGatt(server);
  startValidationAdvertising();
  Serial.println("[val] validation + hybrid GATT ready (automatic HID fallback)");
}

void handleSerial() {
  while (Serial.available()) {
    char c = (char)Serial.read();
    if (c == '\n' || c == '\r') {
      gLine.trim();
      if (gLine.equalsIgnoreCase("help")) {
        Serial.println("val test a | val release | status | hello | reboot");
      } else if (gLine.equalsIgnoreCase("status")) {
        Serial.printf("%s subscribed=%d seq=%u bank=%u preset=%d battery=%u%%\n",
                      FW_INFO, (int)gValSubscribed, (unsigned)gValSeq,
                      (unsigned)gCurrentBank, currentPreset,
                      (unsigned)gBatteryPct);
      } else if (gLine.equalsIgnoreCase("val test a")) {
        uint8_t keys[6] = {0x04, 0, 0, 0, 0, 0};
        sendKeyboardState(0, keys);
        delay(80);
        uint8_t empty[6] = {0};
        sendKeyboardState(0, empty);
      } else if (gLine.equalsIgnoreCase("val release")) {
        sendReleaseAll();
      } else if (gLine.equalsIgnoreCase("hello")) {
        sendHello();
      } else if (gLine.equalsIgnoreCase("reboot")) {
        sendReleaseAll();
        delay(30);
        ESP.restart();
      } else if (gLine.length()) {
        Serial.printf("unknown: %s\n", gLine.c_str());
      }
      gLine = "";
    } else if (gLine.length() < 64) {
      gLine += c;
    }
  }
}

void setup() {
  Serial.begin(115200);
#if CYBERPAD_USB_DIAGNOSTIC
  Serial.setDebugOutput(true);
  // Never let a diagnostic print block the loop. HWCDC stalls on write when no
  // host is draining the CDC buffer, and a 1 Hz heartbeat stalling the loop
  // starves the BLE work that keeps the S3 dongle subscribed. Diagnostics must
  // never be able to break the thing they are diagnosing.
  Serial.setTxTimeoutMs(0);
  delay(1500);
  const esp_reset_reason_t resetReason = esp_reset_reason();
  Serial.printf("[diag] boot reset_reason=%d name=%s\n", (int)resetReason,
                diagResetReasonName(resetReason));
#endif
  pinMode(BUTTON1, INPUT_PULLUP);
  pinMode(BUTTON2, INPUT_PULLUP);
  pinMode(BUTTON3, INPUT_PULLUP);
  pinMode(BUTTON4, INPUT_PULLUP);
  pinMode(BUTTON5, INPUT_PULLUP);
  // GPIO12 (green preset LED / USB D-) is arbitrated in loop() over a grace
  // window, not here -- see pad_led_guard.h. USB keeps the pin until proven
  // absent, so nothing drives it during setup.
  padLedPinMode(LED_GREEN);
  pinMode(LED_RED, OUTPUT);
  pinMode(LED_BLUE, OUTPUT);
  pinMode(BATTERY_PIN, INPUT);
  analogReadResolution(12);
  // Arduino's historical ADC_11db name selects the ESP32-C6 12 dB range.
  analogSetPinAttenuation(BATTERY_PIN, ADC_11db);
  padLedWrite(LED_RED, true);
  delay(100);
  padLedWrite(LED_RED, false);
#if CYBERPAD_USB_DIAGNOSTIC
  Serial.println("[diag] stage=gpio-ready");
#endif
  loadConfig();
#if CYBERPAD_USB_DIAGNOSTIC
  Serial.println("[diag] stage=config-loaded");
#endif
  sampleBattery(true);
#if CYBERPAD_USB_DIAGNOSTIC
  Serial.println("[diag] stage=battery-sampled");
#endif
  currentPreset = 1;
  updatePresetLeds();
#if CYBERPAD_USB_DIAGNOSTIC
  Serial.println("[diag] stage=before-gatt");
#endif
  setupValidationGatt();
#if CYBERPAD_USB_DIAGNOSTIC
  Serial.println("[diag] stage=gatt-ready");
#endif
  connNeoOff();
  Serial.println(FW_INFO);
  Serial.println("Default: S3 dongle path (validation GATT). Long-press B1 = BlueZ HID fallback.");
  Serial.println("B2/B4/B5 = actions · B3 short=bank/long=lights · B1 short=preset");
  Serial.println("[neo] bank colour=linked · blue flash=alone · low battery=pulse");
}

void loop() {
  // Fail-safe GPIO12 arbitration: USB keeps the pin until a full grace window
  // passes with no host ever seen. Claims the green preset LED at most once.
  if (padLedArbitrateTick()) {
    padLedPinMode(LED_GREEN);
    updatePresetLeds();
#if CYBERPAD_USB_DIAGNOSTIC
    Serial.println("[diag] gpio12 owner=green-preset-led (no USB host seen)");
#endif
  }

#if CYBERPAD_USB_DIAGNOSTIC
  static uint32_t lastDiagHeartbeatMs = 0;
  static uint32_t diagLoopCount = 0;
  diagLoopCount++;
  if (millis() - lastDiagHeartbeatMs >= 1000) {
    lastDiagHeartbeatMs = millis();
    // loops/s matters: a slow loop silently misses button edges entirely.
    Serial.printf("[diag] alive ms=%lu loops=%lu b3=%d val_sub=%d hid=%d macro_sub=%d bank=%u\n",
                  (unsigned long)lastDiagHeartbeatMs, (unsigned long)diagLoopCount,
                  (int)digitalRead(BUTTON3), (int)gValSubscribed,
                  (int)keyboard.isConnected(), (int)gattMacroSubscribed,
                  (unsigned)gCurrentBank);
    diagLoopCount = 0;
  }
#endif
  processValidationSubscription();

  // A yanked dongle dies without writing its CCCD, so onSubscribe never fires
  // with 0 and gValSubscribed would stay true forever — blocking the adv kick,
  // direct HID, and the LED. Connection count is the ground truth.
  NimBLEServer *srv = NimBLEDevice::getServer();
  if (gValSubscribed && srv && srv->getConnectedCount() == 0) {
    gValSubscribed = false;
    gHeldAction = -1;
    Serial.println("[val] peer gone — cleared subscription");
  }

  // Solid means any central is attached, S3 or BlueZ alike; the subscription
  // check above keeps this from latching on after a peer vanishes.
  const bool bleLinked = gValSubscribed || keyboard.isConnected();
  // Fallback always fast-blinks so the B1 long-press is visible either way.
  connNeoUpdate(bleLinked, lightsEnabled, gBluezFallback,
                gBluezFallback ? CONN_NEO_FLASH_FAST_MS : CONN_NEO_FLASH_MS,
                BANK_COLOURS[gCurrentBank], gBatteryPct < BAT_LOW_PCT);
  handleSerial();
  sampleBattery(false);

  bool b1 = digitalRead(BUTTON1);
  bool b2 = digitalRead(BUTTON2);
  bool b3 = digitalRead(BUTTON3);
  bool b4 = digitalRead(BUTTON4);
  bool b5 = digitalRead(BUTTON5);

  // B1: short = preset cycle; long-hold = toggle BlueZ HID fallback.
  if (lastButton1 == HIGH && b1 == LOW) {
    delay(40);
    if (digitalRead(BUTTON1) == LOW) {
      gB1DownAtMs = millis();
      gB1LongHandled = false;
    }
  }
  if (b1 == LOW && gB1DownAtMs != 0 && !gB1LongHandled) {
    if ((millis() - gB1DownAtMs) >= BLUEZ_FALLBACK_HOLD_MS) {
      gB1LongHandled = true;
      setBluezFallback(!gBluezFallback);
    }
  }
  if (lastButton1 == LOW && b1 == HIGH) {
    if (!gB1LongHandled && gB1DownAtMs != 0) {
      currentPreset++;
      if (currentPreset > PRESET_COUNT) currentPreset = 1;
      updatePresetLeds();
    }
    gB1DownAtMs = 0;
    gB1LongHandled = false;
  }

  // B3: short = bank cycle; long-hold = indicator-light toggle.
  if (lastButton3 == HIGH && b3 == LOW) {
    delay(40);
    const bool settled = digitalRead(BUTTON3) == LOW;
#if CYBERPAD_USB_DIAGNOSTIC
    Serial.printf("[diag] b3 down settled=%d\n", (int)settled);
#endif
    if (settled) {
      gB3DownAtMs = millis();
      gB3LongHandled = false;
    }
  }
  if (b3 == LOW && gB3DownAtMs != 0 && !gB3LongHandled) {
    if ((millis() - gB3DownAtMs) >= BLUEZ_FALLBACK_HOLD_MS) {
      gB3LongHandled = true;
#if CYBERPAD_USB_DIAGNOSTIC
      Serial.println("[diag] b3 long -> lights toggle");
#endif
      lightsEnabled = !lightsEnabled;
      updatePresetLeds();
      sendLightsState();
    }
  }
  if (lastButton3 == LOW && b3 == HIGH) {
#if CYBERPAD_USB_DIAGNOSTIC
    Serial.printf("[diag] b3 up held=%lums long=%d armed=%d\n",
                  gB3DownAtMs ? (unsigned long)(millis() - gB3DownAtMs) : 0UL,
                  (int)gB3LongHandled, (int)(gB3DownAtMs != 0));
#endif
    if (!gB3LongHandled && gB3DownAtMs != 0) {
      setCurrentBank((gCurrentBank + 1) % BANK_COUNT, true);
    }
    gB3DownAtMs = 0;
    gB3LongHandled = false;
  }

  if (lastButton2 == HIGH && b2 == LOW) {
    delay(40);
    if (digitalRead(BUTTON2) == LOW) pressAction(0);
  }
  if (lastButton2 == LOW && b2 == HIGH && gHeldAction == 0) releaseHeldAction();

  if (lastButton4 == HIGH && b4 == LOW) {
    delay(40);
    if (digitalRead(BUTTON4) == LOW) pressAction(1);
  }
  if (lastButton4 == LOW && b4 == HIGH && gHeldAction == 1) releaseHeldAction();

  if (lastButton5 == HIGH && b5 == LOW) {
    delay(40);
    if (digitalRead(BUTTON5) == LOW) pressAction(2);
  }
  if (lastButton5 == LOW && b5 == HIGH && gHeldAction == 2) releaseHeldAction();

  lastButton1 = b1;
  lastButton2 = b2;
  lastButton3 = b3;
  lastButton4 = b4;
  lastButton5 = b5;

  if (!gBluezFallback && gValSubscribed && (millis() - gLastHeartbeatMs) >= 2000) {
    sendHeartbeat();
    gLastHeartbeatMs = millis();
  }

  if (!gValSubscribed && !keyboard.isConnected() &&
      (millis() - gLastAdvKickMs) >= 3000) {
    if (!NimBLEDevice::getAdvertising()->isAdvertising()) {
      Serial.println("[val] adv inactive — kick");
      startValidationAdvertising();
    }
    gLastAdvKickMs = millis();
  }
}

#else
// ═══════════════════════════════════════════════════════════════════════════
// Default: hybrid BLE HID + Cyberdeck GATT (production-equivalent path)
// ═══════════════════════════════════════════════════════════════════════════

static const char *CYBERDECK_SERVICE_UUID   = "c0de0001-3d17-4a00-8000-00805f9b34fb";
static const char *CYBERDECK_SLOTS_UUID     = "c0de0002-3d17-4a00-8000-00805f9b34fb";
static const char *CYBERDECK_MACRO_EVT_UUID = "c0de0003-3d17-4a00-8000-00805f9b34fb";
static const char *CYBERDECK_INFO_UUID      = "c0de0004-3d17-4a00-8000-00805f9b34fb";
static const char *FW_INFO = "Cyberdeck Pad Hybrid v0.2.0";

HijelHID_BLEKeyboard keyboard("Cyberdeck Pad", "Stitch", 100);
Preferences prefs;

#if ENABLE_WIFI_FALLBACK
const unsigned long CONFIG_HOLD_MS = 1500;
const char *CONFIG_AP_SSID     = "CyberdeckPad-Config";
const char *CONFIG_AP_PASSWORD = "changeme123";
WebServer server(80);
bool configMode = false;
unsigned long b3DownAt  = 0;
bool          b3Handled = false;
unsigned long lastBlinkMs = 0;
bool          blinkOn = false;
#endif

static const uint8_t MODE_HID   = 0;
static const uint8_t MODE_MACRO = 1;

struct Hotkey {
  uint8_t mode;
  uint8_t mod;
  uint8_t key;
  char    label[24];
};

static const size_t SLOT_BYTES = sizeof(Hotkey);
static const size_t SLOTS_BYTES = SLOT_BYTES * PRESET_COUNT * ACTION_COUNT;

Hotkey hotkeys[PRESET_COUNT][ACTION_COUNT];
NimBLECharacteristic *pSlotsChar    = nullptr;
NimBLECharacteristic *pMacroEvtChar = nullptr;
bool gattMacroSubscribed = false;

void setDefaults() {
  hotkeys[0][0] = { MODE_HID, 0,                              KEY_RETURN, "Enter" };
  hotkeys[0][1] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_S,      "Ctrl+S (Save)" };
  hotkeys[0][2] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_V,      "Ctrl+V (Paste)" };
  hotkeys[1][0] = { MODE_HID, 0,                              KEY_RETURN, "Enter" };
  hotkeys[1][1] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LSHIFT, KEY_C,      "Ctrl+Shift+C" };
  hotkeys[1][2] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LSHIFT, KEY_V,      "Ctrl+Shift+V" };
  hotkeys[2][0] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LALT,   KEY_1,      "Ctrl+Alt+1" };
  hotkeys[2][1] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LALT,   KEY_2,      "Ctrl+Alt+2" };
  hotkeys[2][2] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LALT,   KEY_3,      "Ctrl+Alt+3" };
  hotkeys[3][0] = { MODE_HID, 0,                              KEY_RETURN, "Enter" };
  hotkeys[3][1] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_C,      "Ctrl+C" };
  hotkeys[3][2] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_X,      "Ctrl+X" };
  hotkeys[4][0] = { MODE_HID, 0,                              KEY_TAB,    "Tab" };
  hotkeys[4][1] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_Z,      "Ctrl+Z" };
  hotkeys[4][2] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LSHIFT, KEY_Z,      "Ctrl+Shift+Z" };
  hotkeys[5][0] = { MODE_HID, KEY_MOD_LALT,                   KEY_TAB,    "Alt+Tab" };
  hotkeys[5][1] = { MODE_HID, 0,                              KEY_ESCAPE, "Esc" };
  hotkeys[5][2] = { MODE_HID, KEY_MOD_LGUI,                   KEY_L,      "Gui+L" };
}

void packSlots(uint8_t *out) {
  size_t i = 0;
  for (int p = 0; p < PRESET_COUNT; p++) {
    for (int a = 0; a < ACTION_COUNT; a++) {
      memcpy(out + i, &hotkeys[p][a], SLOT_BYTES);
      i += SLOT_BYTES;
    }
  }
}

void unpackSlots(const uint8_t *in, size_t len) {
  if (len < SLOTS_BYTES) return;
  size_t i = 0;
  for (int p = 0; p < PRESET_COUNT; p++) {
    for (int a = 0; a < ACTION_COUNT; a++) {
      Hotkey h;
      memcpy(&h, in + i, SLOT_BYTES);
      i += SLOT_BYTES;
      if (h.mode > MODE_MACRO) h.mode = MODE_HID;
      h.label[sizeof(h.label) - 1] = '\0';
      hotkeys[p][a] = h;
    }
  }
}

void saveConfig() { prefs.putBytes("slots", hotkeys, sizeof(hotkeys)); }

void loadConfig() {
  prefs.begin("hotkeys3", false);
  size_t got = prefs.getBytes("slots", hotkeys, sizeof(hotkeys));
  if (got != sizeof(hotkeys)) {
    setDefaults();
    saveConfig();
  }
}

void refreshSlotsCharacteristic() {
  if (!pSlotsChar) return;
  uint8_t buf[SLOTS_BYTES];
  packSlots(buf);
  pSlotsChar->setValue(buf, SLOTS_BYTES);
}

void updatePresetLeds() {
#if ENABLE_WIFI_FALLBACK
  if (configMode) return;
#endif
  if (!lightsEnabled) {
    padLedDigitalWrite(LED_RED, false);
    padLedDigitalWrite(LED_GREEN, false);
    padLedDigitalWrite(LED_BLUE, false);
    return;
  }
  bool r = false, g = false, b = false;
  switch (currentPreset) {
    case 1: r = true; break;
    case 2: g = true; break;
    case 3: b = true; break;
    case 4: r = true; g = true; break;
    case 5: g = true; b = true; break;
    case 6: r = true; b = true; break;
    default: break;
  }
  padLedDigitalWrite(LED_RED,   r);
  padLedDigitalWrite(LED_GREEN, g);
  padLedDigitalWrite(LED_BLUE,  b);
}

void toggleLights() {
  lightsEnabled = !lightsEnabled;
  updatePresetLeds();
}

void blinkNoSubscriber() {
  digitalWrite(LED_BLUE, HIGH);
  delay(80);
  digitalWrite(LED_BLUE, LOW);
  delay(40);
  digitalWrite(LED_BLUE, HIGH);
  delay(80);
  updatePresetLeds();
}

class SlotsCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic *pChar, NimBLEConnInfo &connInfo) override {
    (void)connInfo;
    std::string val = pChar->getValue();
    if (val.size() >= SLOTS_BYTES) {
      unpackSlots(reinterpret_cast<const uint8_t *>(val.data()), val.size());
      saveConfig();
      refreshSlotsCharacteristic();
    }
  }
  // No onRead refresh -- setValue() during an ATT long read corrupts the
  // continuation chunks. See the experimental branch's SlotsCallbacks.
};

class MacroEvtCallbacks : public NimBLECharacteristicCallbacks {
  void onSubscribe(NimBLECharacteristic * /*pChar*/, NimBLEConnInfo & /*connInfo*/,
                   uint16_t subValue) override {
    gattMacroSubscribed = (subValue != 0);
  }
};

SlotsCallbacks slotsCb;
MacroEvtCallbacks macroEvtCb;

void setupCyberdeckGatt() {
  NimBLEServer *server = NimBLEDevice::getServer();
  if (!server) return;
  NimBLEDevice::setMTU(517);
  NimBLEService *svc = server->createService(CYBERDECK_SERVICE_UUID);
  pSlotsChar = svc->createCharacteristic(
      CYBERDECK_SLOTS_UUID,
      NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::WRITE | NIMBLE_PROPERTY::WRITE_NR);
  pSlotsChar->setCallbacks(&slotsCb);
  refreshSlotsCharacteristic();
  pMacroEvtChar = svc->createCharacteristic(
      CYBERDECK_MACRO_EVT_UUID, NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::NOTIFY);
  pMacroEvtChar->setCallbacks(&macroEvtCb);
  uint8_t zero[2] = {0, 0};
  pMacroEvtChar->setValue(zero, 2);
  NimBLECharacteristic *pInfo =
      svc->createCharacteristic(CYBERDECK_INFO_UUID, NIMBLE_PROPERTY::READ);
  pInfo->setValue(FW_INFO);
  svc->start();
  NimBLEAdvertising *pAdv = NimBLEDevice::getAdvertising();
  pAdv->addServiceUUID(CYBERDECK_SERVICE_UUID);
  if (!keyboard.isConnected()) {
    NimBLEDevice::stopAdvertising();
    NimBLEDevice::startAdvertising();
  }
}

void notifyMacroEvent(uint8_t presetIdx, uint8_t actionIdx) {
  if (!pMacroEvtChar) return;
  uint8_t payload[2] = {presetIdx, actionIdx};
  pMacroEvtChar->setValue(payload, 2);
  bool ok = pMacroEvtChar->notify();
  if (!ok || !gattMacroSubscribed) blinkNoSubscriber();
}

void executeHotkey(int presetIdx, int actionIdx) {
  if (presetIdx < 0 || presetIdx >= PRESET_COUNT || actionIdx < 0 ||
      actionIdx >= ACTION_COUNT)
    return;
  Hotkey &h = hotkeys[presetIdx][actionIdx];
  if (h.mode == MODE_MACRO) {
    notifyMacroEvent((uint8_t)presetIdx, (uint8_t)actionIdx);
    return;
  }
  if (h.key == KEY_NONE) return;
  if (!keyboard.isPaired()) return;
  keyboard.tap(h.key, h.mod);
  delay(80);
}

void handleButtonPress(int button) {
  if (button == 1) {
    currentPreset++;
    if (currentPreset > PRESET_COUNT) currentPreset = 1;
    updatePresetLeds();
    return;
  }
  if (button == 2) {
    executeHotkey(currentPreset - 1, 0);
    return;
  }
  if (button == 4) {
    executeHotkey(currentPreset - 1, 1);
    return;
  }
  if (button == 5) {
    executeHotkey(currentPreset - 1, 2);
    return;
  }
}

void setup() {
  pinMode(BUTTON1, INPUT_PULLUP);
  pinMode(BUTTON2, INPUT_PULLUP);
  pinMode(BUTTON3, INPUT_PULLUP);
  pinMode(BUTTON4, INPUT_PULLUP);
  pinMode(BUTTON5, INPUT_PULLUP);
  // GPIO12 (green preset LED / USB D-) is arbitrated in loop() over a grace
  // window, not here -- see pad_led_guard.h.
  padLedPinMode(LED_GREEN);
  pinMode(LED_RED, OUTPUT);
  pinMode(LED_BLUE, OUTPUT);
  padLedDigitalWrite(LED_RED, true);
  delay(150);
  padLedDigitalWrite(LED_GREEN, true);
  delay(150);
  padLedDigitalWrite(LED_BLUE, true);
  delay(150);
  padLedDigitalWrite(LED_RED, false);
  padLedDigitalWrite(LED_GREEN, false);
  padLedDigitalWrite(LED_BLUE, false);
  loadConfig();
  currentPreset = 1;
  updatePresetLeds();
  keyboard.begin();
  setupCyberdeckGatt();
}

void loop() {
  // Fail-safe GPIO12 arbitration -- see pad_led_guard.h.
  if (padLedArbitrateTick()) {
    padLedPinMode(LED_GREEN);
    updatePresetLeds();
  }

  bool b1 = digitalRead(BUTTON1);
  bool b2 = digitalRead(BUTTON2);
  bool b3 = digitalRead(BUTTON3);
  bool b4 = digitalRead(BUTTON4);
  bool b5 = digitalRead(BUTTON5);

  if (lastButton1 == HIGH && b1 == LOW) {
    delay(40);
    if (digitalRead(BUTTON1) == LOW) handleButtonPress(1);
  }
  if (lastButton2 == HIGH && b2 == LOW) {
    delay(40);
    if (digitalRead(BUTTON2) == LOW) handleButtonPress(2);
  }
  if (lastButton4 == HIGH && b4 == LOW) {
    delay(40);
    if (digitalRead(BUTTON4) == LOW) handleButtonPress(4);
  }
  if (lastButton5 == HIGH && b5 == LOW) {
    delay(40);
    if (digitalRead(BUTTON5) == LOW) handleButtonPress(5);
  }
  if (lastButton3 == HIGH && b3 == LOW) {
    delay(40);
    if (digitalRead(BUTTON3) == LOW) toggleLights();
  }

  lastButton1 = b1;
  lastButton2 = b2;
  lastButton3 = b3;
  lastButton4 = b4;
  lastButton5 = b5;
}

#endif // CYBERPAD_EXPERIMENTAL_S3_DONGLE
