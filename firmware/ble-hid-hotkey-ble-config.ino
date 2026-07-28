/**
 * ble-hid-hotkey-ble-config.ino
 *
 * Cyberdeck Pad — Hybrid BLE HID + custom GATT for Macro Command Center.
 *
 * Foundation: ble-hid-hotkey.ino.ino / ble-hid-hotkey-wifi.ino
 *
 * Each of the 18 action slots (6 presets x B2/B4/B5) has a mode:
 *   MODE_HID   (0) — tap key+mods via BLE HID (works with app closed)
 *   MODE_MACRO (1) — notify GATT MacroEvent {preset, actionIdx}; desktop runs it
 *
 * Preset LEDs (3 physical LEDs → 6 presets):
 *   P1 Red · P2 Green · P3 Blue
 *   P4 Red+Green · P5 Green+Blue · P6 Red+Blue  (dual solid)
 *
 * Custom GATT (same BLE link as HID — no second connection):
 *   Service  c0de0001-3d17-4a00-8000-00805f9b34fb
 *   Slots    c0de0002-...  read/write 18 packed slots (486 bytes)
 *   MacroEvt c0de0003-...  notify 2 bytes [presetIdx, actionIdx]
 *   Info     c0de0004-...  read firmware id string
 *
 * Optional WiFi config portal (rescue): compile with -DENABLE_WIFI_FALLBACK=1
 * Default: WiFi off. Long-press B3 only toggles LEDs when WiFi fallback is off.
 *
 * Library: HijelHID_BLEKeyboard (NimBLE-Arduino >= 2.3.8, ESP32 core >= 3.3.7).
 */

#include <Arduino.h>
#include <HijelHID_BLEKeyboard.h>
#include <NimBLEDevice.h>
#include <Preferences.h>

#ifndef ENABLE_WIFI_FALLBACK
#define ENABLE_WIFI_FALLBACK 0
#endif

#if ENABLE_WIFI_FALLBACK
#include <WiFi.h>
#include <WebServer.h>
#endif

// ─── GATT UUIDs (must match desktop protocol) ───────────────────────────────
static const char* CYBERDECK_SERVICE_UUID   = "c0de0001-3d17-4a00-8000-00805f9b34fb";
static const char* CYBERDECK_SLOTS_UUID     = "c0de0002-3d17-4a00-8000-00805f9b34fb";
static const char* CYBERDECK_MACRO_EVT_UUID = "c0de0003-3d17-4a00-8000-00805f9b34fb";
static const char* CYBERDECK_INFO_UUID      = "c0de0004-3d17-4a00-8000-00805f9b34fb";

static const char* FW_INFO = "Cyberdeck Pad Hybrid v0.2.0";

HijelHID_BLEKeyboard keyboard("Cyberdeck Pad", "Stitch", 100);
Preferences prefs;

// ─── Buttons / LEDs ─────────────────────────────────────────────────────────
const int BUTTON1 = 2;
const int BUTTON2 = 3;
const int BUTTON3 = 4;
const int BUTTON4 = 6;
const int BUTTON5 = 5;

const int LED_GREEN = 12; // Preset 2 / pairs
const int LED_RED   = 21; // Preset 1 / pairs
const int LED_BLUE  = 15; // Preset 3 / pairs

static const int PRESET_COUNT = 6;
static const int ACTION_COUNT = 3;

int  currentPreset = 1; // 1..PRESET_COUNT (human)
bool lightsEnabled = true;

bool lastButton1 = HIGH;
bool lastButton2 = HIGH;
bool lastButton3 = HIGH;
bool lastButton4 = HIGH;
bool lastButton5 = HIGH;

#if ENABLE_WIFI_FALLBACK
const unsigned long CONFIG_HOLD_MS = 1500;
const char* CONFIG_AP_SSID     = "CyberdeckPad-Config";
const char* CONFIG_AP_PASSWORD = "changeme123";
WebServer server(80);
bool configMode = false;
unsigned long b3DownAt  = 0;
bool          b3Handled = false;
unsigned long lastBlinkMs = 0;
bool          blinkOn = false;
#endif

// ─── Hotkey model ───────────────────────────────────────────────────────────
static const uint8_t MODE_HID   = 0;
static const uint8_t MODE_MACRO = 1;

struct Hotkey {
  uint8_t mode;       // MODE_HID | MODE_MACRO
  uint8_t mod;
  uint8_t key;
  char    label[24];
};

static const size_t SLOT_BYTES = sizeof(Hotkey); // 27
static const size_t SLOTS_BYTES = SLOT_BYTES * PRESET_COUNT * ACTION_COUNT; // 486

Hotkey hotkeys[PRESET_COUNT][ACTION_COUNT];

NimBLECharacteristic* pSlotsChar    = nullptr;
NimBLECharacteristic* pMacroEvtChar = nullptr;
bool gattMacroSubscribed = false;

// ─── Defaults (match original firmware HID behavior) ────────────────────────
void setDefaults() {
  hotkeys[0][0] = { MODE_HID, 0,                              KEY_RETURN, "Enter" };
  hotkeys[0][1] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_S,      "Ctrl+S (Save)" };
  hotkeys[0][2] = { MODE_HID, KEY_MOD_LCTRL,                  KEY_V,      "Ctrl+V (Paste)" };
  hotkeys[1][0] = { MODE_HID, 0,                              KEY_RETURN, "Enter" };
  hotkeys[1][1] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LSHIFT, KEY_C,      "Ctrl+Shift+C" };
  hotkeys[1][2] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LSHIFT, KEY_V,      "Ctrl+Shift+V" };
  // Preset 3 — host-bridge chords (MCC maps actions on desktop)
  hotkeys[2][0] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LALT,   KEY_1,      "Ctrl+Alt+1" };
  hotkeys[2][1] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LALT,   KEY_2,      "Ctrl+Alt+2" };
  hotkeys[2][2] = { MODE_HID, KEY_MOD_LCTRL | KEY_MOD_LALT,   KEY_3,      "Ctrl+Alt+3" };
  // Presets 4–6 — starter HID (reconfigure via MCC)
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

void packSlots(uint8_t* out) {
  size_t i = 0;
  for (int p = 0; p < PRESET_COUNT; p++) {
    for (int a = 0; a < ACTION_COUNT; a++) {
      memcpy(out + i, &hotkeys[p][a], SLOT_BYTES);
      i += SLOT_BYTES;
    }
  }
}

void unpackSlots(const uint8_t* in, size_t len) {
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

void saveConfig() {
  prefs.putBytes("slots", hotkeys, sizeof(hotkeys));
}

void loadConfig() {
  // hotkeys3 — 6 presets (486 bytes). Older namespaces stay untouched.
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

// ─── LEDs ───────────────────────────────────────────────────────────────────
// P1 R · P2 G · P3 B · P4 R+G · P5 G+B · P6 R+B
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

// ─── GATT callbacks ─────────────────────────────────────────────────────────
class SlotsCallbacks : public NimBLECharacteristicCallbacks {
  void onWrite(NimBLECharacteristic* pChar, NimBLEConnInfo& connInfo) override {
    (void)connInfo;
    std::string val = pChar->getValue();
    if (val.size() >= SLOTS_BYTES) {
      unpackSlots(reinterpret_cast<const uint8_t*>(val.data()), val.size());
      saveConfig();
      refreshSlotsCharacteristic();
    }
  }
  void onRead(NimBLECharacteristic* pChar, NimBLEConnInfo& connInfo) override {
    (void)connInfo;
    refreshSlotsCharacteristic();
  }
};

class MacroEvtCallbacks : public NimBLECharacteristicCallbacks {
  void onSubscribe(NimBLECharacteristic* pChar, NimBLEConnInfo& connInfo, uint16_t subValue) override {
    (void)pChar;
    (void)connInfo;
    // 0 = unsubscribed, 1 = notify, 2 = indicate, 3 = both
    gattMacroSubscribed = (subValue != 0);
  }
};

SlotsCallbacks slotsCb;
MacroEvtCallbacks macroEvtCb;

void setupCyberdeckGatt() {
  NimBLEServer* server = NimBLEDevice::getServer();
  if (!server) return;

  // Larger MTU helps 486-byte Slots R/W without many prepare-writes.
  NimBLEDevice::setMTU(517);

  NimBLEService* svc = server->createService(CYBERDECK_SERVICE_UUID);

  pSlotsChar = svc->createCharacteristic(
      CYBERDECK_SLOTS_UUID,
      NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::WRITE | NIMBLE_PROPERTY::WRITE_NR);
  pSlotsChar->setCallbacks(&slotsCb);
  refreshSlotsCharacteristic();

  pMacroEvtChar = svc->createCharacteristic(
      CYBERDECK_MACRO_EVT_UUID,
      NIMBLE_PROPERTY::READ | NIMBLE_PROPERTY::NOTIFY);
  pMacroEvtChar->setCallbacks(&macroEvtCb);
  uint8_t zero[2] = {0, 0};
  pMacroEvtChar->setValue(zero, 2);

  NimBLECharacteristic* pInfo = svc->createCharacteristic(
      CYBERDECK_INFO_UUID,
      NIMBLE_PROPERTY::READ);
  pInfo->setValue(FW_INFO);

  svc->start();

  // Advertise custom service so BlueZ / desktop can discover it on the HID link.
  NimBLEAdvertising* pAdv = NimBLEDevice::getAdvertising();
  pAdv->addServiceUUID(CYBERDECK_SERVICE_UUID);
  // Restart advertising if not connected so the new UUID is visible.
  if (!keyboard.isConnected()) {
    NimBLEDevice::stopAdvertising();
    NimBLEDevice::startAdvertising();
  }
}

void notifyMacroEvent(uint8_t presetIdx, uint8_t actionIdx) {
  if (!pMacroEvtChar) return;
  uint8_t payload[2] = { presetIdx, actionIdx };
  pMacroEvtChar->setValue(payload, 2);
  // Always attempt notify. Do not gate on our subscribe flag — BlueZ/NimBLE
  // CCCD handling can miss onSubscribe while the host is still able to receive.
  bool ok = pMacroEvtChar->notify();
  if (!ok || !gattMacroSubscribed) {
    // Visual feedback when nobody is clearly listening / notify failed.
    blinkNoSubscriber();
  }
}

// ─── Button actions ─────────────────────────────────────────────────────────
void executeHotkey(int presetIdx, int actionIdx) {
  if (presetIdx < 0 || presetIdx >= PRESET_COUNT || actionIdx < 0 || actionIdx >= ACTION_COUNT) return;
  Hotkey& h = hotkeys[presetIdx][actionIdx];

  if (h.mode == MODE_MACRO) {
    notifyMacroEvent((uint8_t)presetIdx, (uint8_t)actionIdx);
    return;
  }

  // MODE_HID
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
  if (button == 2) { executeHotkey(currentPreset - 1, 0); return; }
  if (button == 4) { executeHotkey(currentPreset - 1, 1); return; }
  if (button == 5) { executeHotkey(currentPreset - 1, 2); return; }
}

#if ENABLE_WIFI_FALLBACK
// Minimal rescue portal — HID-only edit (no mode field in CSV; mode stays as-is
// unless label/key/mod overwritten). Full hybrid config is via BLE GATT.
void handleGetHotkeys() {
  String j = "[";
  for (int p = 0; p < PRESET_COUNT; p++) {
    for (int a = 0; a < ACTION_COUNT; a++) {
      Hotkey& h = hotkeys[p][a];
      if (j.length() > 1) j += ",";
      j += "{\"p\":"; j += p;
      j += ",\"a\":"; j += a;
      j += ",\"mode\":"; j += h.mode;
      j += ",\"mod\":"; j += h.mod;
      j += ",\"key\":"; j += h.key;
      j += ",\"label\":\"";
      for (int i = 0; i < 24 && h.label[i]; i++) {
        char c = h.label[i];
        if (c == '"' || c == '\\') j += '\\';
        j += c;
      }
      j += "\"}";
    }
  }
  j += "]";
  server.send(200, "application/json", j);
}

void handleSave() {
  String body = server.arg("plain");
  int start = 0;
  while (start < (int)body.length()) {
    int nl = body.indexOf('\n', start);
    if (nl < 0) nl = body.length();
    String line = body.substring(start, nl);
    start = nl + 1;
    line.trim();
    if (line.length() == 0) continue;

    // p,a,mode,mod,key,label
    int c1 = line.indexOf(',');
    int c2 = line.indexOf(',', c1 + 1);
    int c3 = line.indexOf(',', c2 + 1);
    int c4 = line.indexOf(',', c3 + 1);
    int c5 = line.indexOf(',', c4 + 1);
    if (c1 < 0 || c2 < 0 || c3 < 0 || c4 < 0 || c5 < 0) continue;

    int p    = line.substring(0, c1).toInt();
    int a    = line.substring(c1 + 1, c2).toInt();
    int mode = line.substring(c2 + 1, c3).toInt();
    int mod  = line.substring(c3 + 1, c4).toInt();
    int key  = line.substring(c4 + 1, c5).toInt();
    String label = line.substring(c5 + 1);

    if (p < 0 || p >= PRESET_COUNT || a < 0 || a >= ACTION_COUNT) continue;
    Hotkey& h = hotkeys[p][a];
    h.mode = (mode == MODE_MACRO) ? MODE_MACRO : MODE_HID;
    h.mod = (uint8_t)mod;
    h.key = (uint8_t)key;
    label.toCharArray(h.label, sizeof(h.label));
  }
  saveConfig();
  refreshSlotsCharacteristic();
  server.send(200, "text/plain", "OK");
}

void enterConfigMode() {
  configMode = true;
  WiFi.mode(WIFI_AP);
  WiFi.softAP(CONFIG_AP_SSID, CONFIG_AP_PASSWORD);
  server.on("/api/hotkeys", HTTP_GET, handleGetHotkeys);
  server.on("/save", HTTP_POST, handleSave);
  server.on("/", []() {
    server.send(200, "text/plain",
                "Cyberdeck hybrid rescue API. Use GET /api/hotkeys and POST /save");
  });
  server.begin();
  digitalWrite(LED_RED, LOW);
  digitalWrite(LED_GREEN, LOW);
  digitalWrite(LED_BLUE, HIGH);
  lastBlinkMs = millis();
  blinkOn = true;
}

void exitConfigMode() {
  server.stop();
  WiFi.softAPdisconnect(true);
  WiFi.mode(WIFI_OFF);
  configMode = false;
  updatePresetLeds();
}

void toggleConfigMode() {
  if (configMode) exitConfigMode();
  else enterConfigMode();
}
#endif

void setup() {
  pinMode(BUTTON1, INPUT_PULLUP);
  pinMode(BUTTON2, INPUT_PULLUP);
  pinMode(BUTTON3, INPUT_PULLUP);
  pinMode(BUTTON4, INPUT_PULLUP);
  pinMode(BUTTON5, INPUT_PULLUP);

  pinMode(LED_GREEN, OUTPUT);
  pinMode(LED_RED, OUTPUT);
  pinMode(LED_BLUE, OUTPUT);

  digitalWrite(LED_RED, HIGH);   delay(150);
  digitalWrite(LED_GREEN, HIGH); delay(150);
  digitalWrite(LED_BLUE, HIGH);  delay(150);
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
#if ENABLE_WIFI_FALLBACK
  if (configMode) {
    server.handleClient();
    if (millis() - lastBlinkMs > 250) {
      blinkOn = !blinkOn;
      digitalWrite(LED_BLUE, blinkOn ? HIGH : LOW);
      lastBlinkMs = millis();
    }
  }
#endif

  bool b1 = digitalRead(BUTTON1);
  bool b2 = digitalRead(BUTTON2);
  bool b3 = digitalRead(BUTTON3);
  bool b4 = digitalRead(BUTTON4);
  bool b5 = digitalRead(BUTTON5);

  if (lastButton1 == HIGH && b1 == LOW) { delay(40); if (digitalRead(BUTTON1) == LOW) handleButtonPress(1); }
  if (lastButton2 == HIGH && b2 == LOW) { delay(40); if (digitalRead(BUTTON2) == LOW) handleButtonPress(2); }
  if (lastButton4 == HIGH && b4 == LOW) { delay(40); if (digitalRead(BUTTON4) == LOW) handleButtonPress(4); }
  if (lastButton5 == HIGH && b5 == LOW) { delay(40); if (digitalRead(BUTTON5) == LOW) handleButtonPress(5); }

#if ENABLE_WIFI_FALLBACK
  if (lastButton3 == HIGH && b3 == LOW) {
    delay(40);
    if (digitalRead(BUTTON3) == LOW) { b3DownAt = millis(); b3Handled = false; }
  }
  if (b3 == LOW && !b3Handled && (millis() - b3DownAt >= CONFIG_HOLD_MS)) {
    toggleConfigMode();
    b3Handled = true;
  }
  if (lastButton3 == LOW && b3 == HIGH) {
    if (!b3Handled) toggleLights();
    b3Handled = false;
  }
#else
  if (lastButton3 == HIGH && b3 == LOW) {
    delay(40);
    if (digitalRead(BUTTON3) == LOW) toggleLights();
  }
#endif

  lastButton1 = b1;
  lastButton2 = b2;
  lastButton3 = b3;
  lastButton4 = b4;
  lastButton5 = b5;
}
