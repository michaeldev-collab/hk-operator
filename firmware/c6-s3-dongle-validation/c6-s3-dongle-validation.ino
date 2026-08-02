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
static const char *FW_INFO = "Cyberpad C6 S3-Dongle Validation 0.4.6";
static const char *ADV_NAME = "Cyberpad Val C6";

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

HijelHID_BLEKeyboard keyboard(ADV_NAME, "Stitch", 100);
Preferences prefs;
Hotkey hotkeys[PRESET_COUNT][ACTION_COUNT];

NimBLECharacteristic *pValNotify    = nullptr;
NimBLECharacteristic *pSlotsChar    = nullptr;
NimBLECharacteristic *pMacroEvtChar = nullptr;
bool gValSubscribed = false;
bool gattMacroSubscribed = false;
uint16_t gValSeq = 1;
uint32_t gLastHeartbeatMs = 0;
uint32_t gLastAdvKickMs = 0;
int gHeldAction = -1; // 0..2 while HID key held via B2/B4/B5
bool gBluezFallback = false; // long-press B1: BLE HID to host instead of S3 bridge
uint32_t gB1DownAtMs = 0;
bool gB1LongHandled = false;
String gLine;

#ifndef BLUEZ_FALLBACK_HOLD_MS
#define BLUEZ_FALLBACK_HOLD_MS 1200
#endif

/* 75% duty — 25% dimmer than full on, matching CONN_NEO_BRIGHT. */
#ifndef PAD_LED_LEVEL
#define PAD_LED_LEVEL 191
#endif

static inline void padLedWrite(int pin, bool on) {
  analogWrite(pin, on ? PAD_LED_LEVEL : 0);
}


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

void notifyMacroEvent(uint8_t presetIdx, uint8_t actionIdx) {
  if (!pMacroEvtChar) return;
  uint8_t payload[2] = {presetIdx, actionIdx};
  pMacroEvtChar->setValue(payload, 2);
  pMacroEvtChar->notify();
  (void)gattMacroSubscribed;
}

void pressAction(int actionIdx) {
  if (actionIdx < 0 || actionIdx >= ACTION_COUNT) return;
  Hotkey &h = hotkeys[currentPreset - 1][actionIdx];
  if (h.mode == MODE_MACRO) {
    notifyMacroEvent((uint8_t)(currentPreset - 1), (uint8_t)actionIdx);
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
    gValSubscribed = (subValue != 0);
    Serial.printf("[val] subscribe=%u\n", (unsigned)subValue);
    if (gValSubscribed) {
      sendHello();
      sendLightsState();
      gLastHeartbeatMs = millis();
    } else {
      releaseHeldAction();
    }
  }
};

class SlotsCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic *pChar, NimBLEConnInfo &connInfo) override {
    (void)connInfo;
    std::string val = pChar->getValue();
    if (val.size() >= SLOTS_BYTES) {
      unpackSlots(reinterpret_cast<const uint8_t *>(val.data()), val.size());
      saveConfig();
      refreshSlotsCharacteristic();
      Serial.println("[slots] wrote from GATT");
    }
  }
  void onRead(NimBLECharacteristic * /*pChar*/, NimBLEConnInfo & /*connInfo*/) override {
    refreshSlotsCharacteristic();
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

  NimBLEService *svc = server->createService(CPAD_VAL_SERVICE_UUID);
  pValNotify = svc->createCharacteristic(
      CPAD_VAL_NOTIFY_UUID, NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::NOTIFY);
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
        Serial.printf("%s subscribed=%d seq=%u preset=%d\n", FW_INFO,
                      (int)gValSubscribed, (unsigned)gValSeq, currentPreset);
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
  pinMode(BUTTON1, INPUT_PULLUP);
  pinMode(BUTTON2, INPUT_PULLUP);
  pinMode(BUTTON3, INPUT_PULLUP);
  pinMode(BUTTON4, INPUT_PULLUP);
  pinMode(BUTTON5, INPUT_PULLUP);
  pinMode(LED_GREEN, OUTPUT);
  pinMode(LED_RED, OUTPUT);
  pinMode(LED_BLUE, OUTPUT);
  padLedWrite(LED_RED, true);
  delay(100);
  padLedWrite(LED_RED, false);
  loadConfig();
  currentPreset = 1;
  updatePresetLeds();
  setupValidationGatt();
  connNeoOff();
  Serial.println(FW_INFO);
  Serial.println("Default: S3 dongle path (validation GATT). Long-press B1 = BlueZ HID fallback.");
  Serial.println("B2/B4/B5 = slot actions · B3 = lights · short B1 = preset");
  Serial.println("[neo] green solid=BLE linked · slow flash=alone · fast flash=BlueZ waiting");
}

void loop() {
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
                gBluezFallback ? CONN_NEO_FLASH_FAST_MS : CONN_NEO_FLASH_MS);
  handleSerial();

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

  if (lastButton3 == HIGH && b3 == LOW) {
    delay(40);
    if (digitalRead(BUTTON3) == LOW) {
      lightsEnabled = !lightsEnabled;
      updatePresetLeds();
      sendLightsState();
    }
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
    digitalWrite(LED_RED, LOW);
    digitalWrite(LED_GREEN, LOW);
    digitalWrite(LED_BLUE, LOW);
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
  digitalWrite(LED_RED,   r ? HIGH : LOW);
  digitalWrite(LED_GREEN, g ? HIGH : LOW);
  digitalWrite(LED_BLUE,  b ? HIGH : LOW);
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
  void onRead(NimBLECharacteristic * /*pChar*/, NimBLEConnInfo & /*connInfo*/) override {
    refreshSlotsCharacteristic();
  }
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
  pinMode(LED_GREEN, OUTPUT);
  pinMode(LED_RED, OUTPUT);
  pinMode(LED_BLUE, OUTPUT);
  digitalWrite(LED_RED, HIGH);
  delay(150);
  digitalWrite(LED_GREEN, HIGH);
  delay(150);
  digitalWrite(LED_BLUE, HIGH);
  delay(150);
  digitalWrite(LED_RED, LOW);
  digitalWrite(LED_GREEN, LOW);
  digitalWrite(LED_BLUE, LOW);
  loadConfig();
  currentPreset = 1;
  updatePresetLeds();
  keyboard.begin();
  setupCyberdeckGatt();
}

void loop() {
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
