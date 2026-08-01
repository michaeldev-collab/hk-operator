/**
 * Cyberpad S3 Validation Dongle — bench PoC only.
 *
 * USB HID keyboard (TinyUSB) + optional CDC diagnostics + BLE central bridge
 * for the experimental C6 validation GATT service.
 *
 * Build (USB-OTG TinyUSB + CDC):
 *   arduino-cli compile --fqbn \
 *     esp32:esp32:esp32s3:USBMode=default,CDCOnBoot=cdc,FlashSize=4M,PSRAM=disabled \
 *     --libraries firmware \
 *     --libraries /run/media/stitch/data3/Operating/pi-iot/libraries \
 *     firmware/s3-dongle-validation
 *
 * Never types on boot. Never auto-repeats. Explicit commands / BLE reports only.
 */

#include <Arduino.h>

#ifndef ARDUINO_USB_MODE
#error This ESP32 SoC has no Native USB interface
#elif ARDUINO_USB_MODE == 1
#error Build with USBMode=default (USB-OTG TinyUSB), not Hardware CDC/JTAG
#endif

#include "USB.h"
#include "USBHIDKeyboard.h"
#include <NimBLEDevice.h>
#include <validation_protocol.h>
#include <conn_neopixel.h>
#include <cpad_base64.h>
#include <freertos/FreeRTOS.h>
#include <freertos/queue.h>

#define FW_VERSION "s3-dongle-validation 0.4.4"
#define DEVICE_USB_NAME "Cyberpad S3 Validation Dongle"
#define HEARTBEAT_TIMEOUT_MS 5000
#define HID_TEST_HOLD_MS 80
#define SCAN_WINDOW_MS 12000
#define RECONNECT_INTERVAL_MS 2500
#define CDC_LINE_MAX 800
#define HYBRID_SLOTS_BYTES 486
/* PoC-only known Cyberpad address (documented). Prefer UUID scan when possible. */
#define CPAD_VAL_PEER_ADDR "20:6E:F1:11:5F:36"
#define CYBERDECK_SERVICE_UUID "c0de0001-3d17-4a00-8000-00805f9b34fb"
#define CYBERDECK_SLOTS_UUID   "c0de0002-3d17-4a00-8000-00805f9b34fb"
#define CYBERDECK_INFO_UUID    "c0de0004-3d17-4a00-8000-00805f9b34fb"

enum BridgeState : uint8_t {
  ST_BOOT = 0,
  ST_USB_READY,
  ST_SCANNING,
  ST_CONNECTING,
  ST_DISCOVERING,
  ST_SUBSCRIBED,
  ST_CONNECTED,
  ST_DISCONNECTED,
  ST_ERROR,
};

static const char *stateName(BridgeState s) {
  switch (s) {
    case ST_BOOT: return "BOOT";
    case ST_USB_READY: return "USB_READY";
    case ST_SCANNING: return "SCANNING";
    case ST_CONNECTING: return "CONNECTING";
    case ST_DISCOVERING: return "DISCOVERING";
    case ST_SUBSCRIBED: return "SUBSCRIBED";
    case ST_CONNECTED: return "CONNECTED";
    case ST_DISCONNECTED: return "DISCONNECTED";
    case ST_ERROR: return "ERROR";
    default: return "?";
  }
}

USBHIDKeyboard Keyboard;

static BridgeState gState = ST_BOOT;
static QueueHandle_t gPktQueue = nullptr;
static cpad_val_packet_t gLastApplied;
static bool gHaveLast = false;
static uint32_t gLastHeartbeatMs = 0;
static bool gBleWantScan = false;
static bool gBleConnected = false;
static bool gAutoReconnect = true; // keep trying known/scanned peer after drops
static bool gConnectBusy = false;
static uint32_t gLastReconnectAttemptMs = 0;
static NimBLEClient *gClient = nullptr;
static NimBLERemoteCharacteristic *gNotifyChar = nullptr;
static NimBLERemoteCharacteristic *gSlotsChar = nullptr;
static NimBLERemoteCharacteristic *gInfoChar = nullptr;
static NimBLEAddress gPeerAddr;
static bool gHavePeer = false;
static bool gSlotsReady = false;
static bool gLightsEnabled = true; // mirrors pad B3 indicator toggle
static String gLine;

static void setState(BridgeState s) {
  if (gState == s) return;
  gState = s;
  Serial.printf("[state] %s\n", stateName(s));
}

static void usbReleaseAll(const char *reason) {
  KeyReport empty = {};
  Keyboard.sendReport(&empty);
  Keyboard.releaseAll();
  gHaveLast = false;
  memset(&gLastApplied, 0, sizeof(gLastApplied));
  Serial.printf("[hid] release-all (%s)\n", reason ? reason : "?");
}

static bool reportsEqual(const cpad_val_packet_t &a, const cpad_val_packet_t &b) {
  return a.modifiers == b.modifiers && memcmp(a.keys, b.keys, 6) == 0;
}

static void applyKeyboardPacket(const cpad_val_packet_t &pkt, const char *src) {
  if (pkt.msg_type == CPAD_VAL_MSG_RELEASE_ALL ||
      (pkt.msg_type == CPAD_VAL_MSG_KEYBOARD_REPORT &&
       pkt.modifiers == 0 &&
       pkt.keys[0] == 0 && pkt.keys[1] == 0 && pkt.keys[2] == 0 &&
       pkt.keys[3] == 0 && pkt.keys[4] == 0 && pkt.keys[5] == 0)) {
    if (gHaveLast && gLastApplied.modifiers == 0 &&
        gLastApplied.keys[0] == 0 && gLastApplied.keys[1] == 0 &&
        gLastApplied.keys[2] == 0 && gLastApplied.keys[3] == 0 &&
        gLastApplied.keys[4] == 0 && gLastApplied.keys[5] == 0) {
      return; // duplicate empty
    }
    usbReleaseAll(src);
    gLastApplied = pkt;
    gLastApplied.modifiers = 0;
    memset(gLastApplied.keys, 0, 6);
    gHaveLast = true;
    return;
  }

  if (pkt.msg_type != CPAD_VAL_MSG_KEYBOARD_REPORT) return;

  if (gHaveLast && reportsEqual(gLastApplied, pkt)) return;

  KeyReport report = {};
  report.modifiers = pkt.modifiers;
  memcpy(report.keys, pkt.keys, 6);
  Keyboard.sendReport(&report);
  gLastApplied = pkt;
  gHaveLast = true;
  Serial.printf("[hid] report from %s mod=0x%02x keys=%02x %02x %02x %02x %02x %02x seq=%u\n",
                src, pkt.modifiers, pkt.keys[0], pkt.keys[1], pkt.keys[2],
                pkt.keys[3], pkt.keys[4], pkt.keys[5], (unsigned)pkt.seq);
}

static void enqueueValidated(const uint8_t *data, size_t len) {
  cpad_val_packet_t pkt;
  int rc = cpad_val_decode(data, len, &pkt);
  if (rc != 0) {
    Serial.printf("[ble] reject packet rc=%d — release-all\n", rc);
    // Safety: malformed after a held key could strand modifiers.
    cpad_val_packet_t rel;
    cpad_val_encode(&rel, CPAD_VAL_MSG_RELEASE_ALL, 0, 0, nullptr);
    xQueueSend(gPktQueue, &rel, 0);
    return;
  }
  if (pkt.msg_type == CPAD_VAL_MSG_HEARTBEAT || pkt.msg_type == CPAD_VAL_MSG_HELLO) {
    gLastHeartbeatMs = millis();
    Serial.printf("[ble] %s seq=%u\n",
                  pkt.msg_type == CPAD_VAL_MSG_HELLO ? "HELLO" : "HEARTBEAT",
                  (unsigned)pkt.seq);
    return;
  }
  if (pkt.msg_type == CPAD_VAL_MSG_LIGHTS) {
    gLastHeartbeatMs = millis();
    gLightsEnabled = (pkt.modifiers != 0);
    if (!gLightsEnabled) connNeoOff();
    Serial.printf("[ble] LIGHTS %s seq=%u\n",
                  gLightsEnabled ? "on" : "off", (unsigned)pkt.seq);
    return;
  }
  gLastHeartbeatMs = millis();
  if (xQueueSend(gPktQueue, &pkt, 0) != pdTRUE) {
    Serial.println("[ble] queue full — release-all");
    cpad_val_packet_t rel;
    cpad_val_encode(&rel, CPAD_VAL_MSG_RELEASE_ALL, 0, 0, nullptr);
    xQueueOverwrite(gPktQueue, &rel);
  }
}

static void notifyCB(NimBLERemoteCharacteristic * /*c*/, uint8_t *data, size_t len,
                     bool /*isNotify*/) {
  // Keep short — queue for loop().
  enqueueValidated(data, len);
}

class ClientCallbacks : public NimBLEClientCallbacks {
  void onConnect(NimBLEClient * /*pClient*/) override {
    gBleConnected = true;
    setState(ST_DISCOVERING);
    Serial.println("[ble] connected");
  }
  void onDisconnect(NimBLEClient * /*pClient*/, int reason) override {
    gBleConnected = false;
    gNotifyChar = nullptr;
    gSlotsChar = nullptr;
    gInfoChar = nullptr;
    gSlotsReady = false;
    gConnectBusy = false;
    gLastReconnectAttemptMs = millis(); // wait RECONNECT_INTERVAL_MS then retry
    setState(ST_DISCONNECTED);
    Serial.printf("[ble] disconnected reason=%d\n", reason);
    cpad_val_packet_t rel;
    cpad_val_encode(&rel, CPAD_VAL_MSG_RELEASE_ALL, 0, 0, nullptr);
    xQueueSend(gPktQueue, &rel, 0);
  }
};

static ClientCallbacks gClientCb;

class ScanCallbacks : public NimBLEScanCallbacks {
  void onResult(const NimBLEAdvertisedDevice *adv) override {
    const bool uuidHit =
        adv->isAdvertisingService(NimBLEUUID(CPAD_VAL_SERVICE_UUID));
    // Name fallback: UUID may only appear after active-scan response.
    const bool nameHit =
        adv->haveName() && adv->getName().find("Cyberpad Val") != std::string::npos;
    if (!uuidHit && !nameHit) return;
    Serial.printf("[ble] found peer %s rssi=%d uuid=%d name=%s\n",
                  adv->getAddress().toString().c_str(), adv->getRSSI(),
                  (int)uuidHit, adv->haveName() ? adv->getName().c_str() : "");
    gPeerAddr = adv->getAddress();
    gHavePeer = true;
    NimBLEDevice::getScan()->stop();
    setState(ST_CONNECTING);
  }
};

static ScanCallbacks gScanCb;

static bool bleConnectAndSubscribe() {
  if (!gHavePeer) {
    Serial.println("[ble] no peer — scan first");
    return false;
  }
  if (gConnectBusy) return false;
  gConnectBusy = true;
  gAutoReconnect = true;
  setState(ST_CONNECTING);
  if (!gClient) {
    gClient = NimBLEDevice::createClient();
    gClient->setClientCallbacks(&gClientCb, false);
  }
  if (gClient->isConnected()) gClient->disconnect();

  if (!gClient->connect(gPeerAddr)) {
    Serial.println("[ble] connect failed");
    setState(ST_ERROR);
    gConnectBusy = false;
    gLastReconnectAttemptMs = millis();
    return false;
  }

  setState(ST_DISCOVERING);
  NimBLERemoteService *svc = gClient->getService(CPAD_VAL_SERVICE_UUID);
  if (!svc) {
    Serial.println("[ble] validation service missing");
    gClient->disconnect();
    setState(ST_ERROR);
    gConnectBusy = false;
    gLastReconnectAttemptMs = millis();
    return false;
  }
  gNotifyChar = svc->getCharacteristic(CPAD_VAL_NOTIFY_UUID);
  if (!gNotifyChar || !gNotifyChar->canNotify()) {
    Serial.println("[ble] notify char missing");
    gClient->disconnect();
    setState(ST_ERROR);
    gConnectBusy = false;
    gLastReconnectAttemptMs = millis();
    return false;
  }
  if (!gNotifyChar->subscribe(true, notifyCB)) {
    Serial.println("[ble] subscribe failed");
    gClient->disconnect();
    setState(ST_ERROR);
    gConnectBusy = false;
    gLastReconnectAttemptMs = millis();
    return false;
  }

  // Optional hybrid slots GATT for MCC config proxy.
  gSlotsReady = false;
  gSlotsChar = nullptr;
  gInfoChar = nullptr;
  NimBLERemoteService *hyb = gClient->getService(CYBERDECK_SERVICE_UUID);
  if (hyb) {
    gSlotsChar = hyb->getCharacteristic(CYBERDECK_SLOTS_UUID);
    gInfoChar = hyb->getCharacteristic(CYBERDECK_INFO_UUID);
    gSlotsReady = (gSlotsChar != nullptr && gSlotsChar->canRead() &&
                   (gSlotsChar->canWrite() || gSlotsChar->canWriteNoResponse()));
    Serial.printf("[ble] hybrid slots_ready=%d\n", (int)gSlotsReady);
  } else {
    Serial.println("[ble] hybrid Cyberdeck service missing (slots proxy unavailable)");
  }

  gLastHeartbeatMs = millis();
  gConnectBusy = false;
  setState(ST_SUBSCRIBED);
  setState(ST_CONNECTED);
  Serial.println("[ble] subscribed");
  return true;
}

static void bleStartScan() {
  gBleWantScan = true;
  setState(ST_SCANNING);
  NimBLEScan *scan = NimBLEDevice::getScan();
  scan->stop();
  scan->setScanCallbacks(&gScanCb, false);
  scan->setActiveScan(true);
  scan->setInterval(45);
  scan->setWindow(45);
  scan->clearResults();
  scan->start(SCAN_WINDOW_MS, false, true);
  Serial.println("[ble] scan start (UUID + name 'Cyberpad Val')");
}

static bool bleConnectKnownPeer() {
  gPeerAddr = NimBLEAddress(std::string(CPAD_VAL_PEER_ADDR), BLE_ADDR_PUBLIC);
  gHavePeer = true;
  Serial.printf("[ble] using known peer %s\n", CPAD_VAL_PEER_ADDR);
  return bleConnectAndSubscribe();
}

static void printHelp() {
  Serial.println(F("Commands:"));
  Serial.println(F("  help | status | version"));
  Serial.println(F("  usb status | hid test a | hid test enter | hid release-all"));
  Serial.println(F("  ble status | scan start | scan stop | connect | disconnect"));
  Serial.println(F("  reconnect on | reconnect off | peer show | bridge status | reboot"));
  Serial.println(F("  slots read | slots write <b64> | pad info"));
}

static void cmdSlotsRead() {
  if (!gBleConnected || !gSlotsReady || !gSlotsChar) {
    Serial.println("ERR slots not ready");
    return;
  }
  NimBLEAttValue val = gSlotsChar->readValue();
  if (val.size() < HYBRID_SLOTS_BYTES) {
    Serial.printf("ERR slots short len=%u\n", (unsigned)val.size());
    return;
  }
  char b64[700];
  int n = cpad_b64_encode(val.data(), HYBRID_SLOTS_BYTES, b64, sizeof(b64));
  if (n < 0) {
    Serial.println("ERR b64 encode");
    return;
  }
  Serial.print("SLOTS ");
  Serial.println(b64);
}

static void cmdSlotsWrite(const String &b64part) {
  if (!gBleConnected || !gSlotsReady || !gSlotsChar) {
    Serial.println("ERR slots not ready");
    return;
  }
  String b64 = b64part;
  b64.trim();
  if (b64.length() < 4) {
    Serial.println("ERR usage: slots write <base64-486>");
    return;
  }
  uint8_t raw[HYBRID_SLOTS_BYTES];
  int n = cpad_b64_decode(b64.c_str(), b64.length(), raw, sizeof(raw));
  if (n != HYBRID_SLOTS_BYTES) {
    Serial.printf("ERR b64 decode got=%d want=%d\n", n, HYBRID_SLOTS_BYTES);
    return;
  }
  if (!gSlotsChar->writeValue(raw, HYBRID_SLOTS_BYTES, true)) {
    Serial.println("ERR slots ble write");
    return;
  }
  Serial.println("OK");
}

static void cmdPadInfo() {
  if (!gBleConnected || !gInfoChar) {
    Serial.println("ERR pad info unavailable");
    return;
  }
  NimBLEAttValue val = gInfoChar->readValue();
  Serial.print("INFO ");
  Serial.write(val.data(), val.size());
  Serial.println();
}

static void hidTestKey(uint8_t usage, const char *label) {
  uint8_t keys[6] = {usage, 0, 0, 0, 0, 0};
  cpad_val_packet_t down, up;
  cpad_val_encode(&down, CPAD_VAL_MSG_KEYBOARD_REPORT, 0, 0, keys);
  cpad_val_encode(&up, CPAD_VAL_MSG_KEYBOARD_REPORT, 0, 0, nullptr);
  applyKeyboardPacket(down, label);
  delay(HID_TEST_HOLD_MS);
  applyKeyboardPacket(up, "hid-test-release");
  Serial.printf("[hid] test %s complete\n", label);
}

static void handleCommand(String cmd) {
  cmd.trim();
  if (cmd.length() == 0) return;

  // slots write keeps base64 case-sensitive.
  if (cmd.startsWith("slots write ") || cmd.startsWith("SLOTS WRITE ")) {
    String rest = cmd.substring(12);
    rest.trim();
    cmdSlotsWrite(rest);
    return;
  }

  cmd.toLowerCase();

  if (cmd == "help") {
    printHelp();
  } else if (cmd == "version") {
    Serial.println(FW_VERSION);
  } else if (cmd == "status" || cmd == "bridge status") {
    Serial.printf(
        "state=%s ble_connected=%d have_peer=%d auto_reconnect=%d slots_ready=%d "
        "queue=%u transport=dongle\n",
        stateName(gState), (int)gBleConnected, (int)gHavePeer, (int)gAutoReconnect,
        (int)gSlotsReady, (unsigned)uxQueueMessagesWaiting(gPktQueue));
  } else if (cmd == "usb status") {
    Serial.printf("usb ready; HID keyboard active; name=%s\n", DEVICE_USB_NAME);
  } else if (cmd == "hid test a") {
    hidTestKey(0x04, "a");
  } else if (cmd == "hid test enter") {
    hidTestKey(0x28, "enter");
  } else if (cmd == "hid release-all") {
    usbReleaseAll("cmd");
  } else if (cmd == "ble status") {
    Serial.printf("state=%s connected=%d peer=%s\n", stateName(gState),
                  (int)gBleConnected,
                  gHavePeer ? gPeerAddr.toString().c_str() : "(none)");
  } else if (cmd == "scan start") {
    bleStartScan();
  } else if (cmd == "scan stop") {
    NimBLEDevice::getScan()->stop();
    gBleWantScan = false;
    Serial.println("[ble] scan stop");
    if (!gBleConnected) setState(ST_USB_READY);
  } else if (cmd == "connect") {
    gAutoReconnect = true;
    if (!gHavePeer) bleConnectKnownPeer();
    else bleConnectAndSubscribe();
  } else if (cmd == "connect known") {
    gAutoReconnect = true;
    bleConnectKnownPeer();
  } else if (cmd == "reconnect on") {
    gAutoReconnect = true;
    if (!gHavePeer) {
      gPeerAddr = NimBLEAddress(std::string(CPAD_VAL_PEER_ADDR), BLE_ADDR_PUBLIC);
      gHavePeer = true;
    }
    Serial.println("[ble] auto-reconnect on");
  } else if (cmd == "reconnect off") {
    gAutoReconnect = false;
    Serial.println("[ble] auto-reconnect off");
  } else if (cmd == "disconnect") {
    gAutoReconnect = false;
    if (gClient && gClient->isConnected()) gClient->disconnect();
    usbReleaseAll("disconnect-cmd");
  } else if (cmd == "peer show") {
    if (gHavePeer) Serial.printf("peer %s\n", gPeerAddr.toString().c_str());
    else Serial.println("peer (none)");
  } else if (cmd == "slots read") {
    cmdSlotsRead();
  } else if (cmd == "pad info") {
    cmdPadInfo();
  } else if (cmd == "reboot") {
    usbReleaseAll("reboot");
    delay(50);
    ESP.restart();
  } else {
    Serial.printf("unknown cmd: %s\n", cmd.c_str());
  }
}

void setup() {
  Serial.begin(115200);
  delay(200);
  Serial.println();
  Serial.println(FW_VERSION);
  Serial.println(DEVICE_USB_NAME);

  gPktQueue = xQueueCreate(8, sizeof(cpad_val_packet_t));
  memset(&gLastApplied, 0, sizeof(gLastApplied));

  // Central-only: do not advertise as peripheral (interferes with scanning).
  NimBLEDevice::init("");
  NimBLEDevice::setPower(ESP_PWR_LVL_P9);

  Keyboard.begin();
  USB.begin();
  // Intentionally no keystrokes here.
  connNeoOff();
  setState(ST_USB_READY);
  // Boot with known peer + auto-reconnect so drops recover without CDC.
  gPeerAddr = NimBLEAddress(std::string(CPAD_VAL_PEER_ADDR), BLE_ADDR_PUBLIC);
  gHavePeer = true;
  gAutoReconnect = true;
  gLastReconnectAttemptMs = 0; // try immediately in loop
  printHelp();
  Serial.println("[ble] auto-reconnect on; known peer 20:6E:F1:11:5F:36");
  Serial.println("[neo] green solid=BLE connected, flash=disconnected (GPIO48)");
}

void loop() {
  connNeoUpdate(gBleConnected, gLightsEnabled, false, CONN_NEO_FLASH_MS);

  while (Serial.available()) {
    char c = (char)Serial.read();
    if (c == '\n' || c == '\r') {
      if (gLine.length()) {
        handleCommand(gLine);
        gLine = "";
      }
    } else if (gLine.length() < CDC_LINE_MAX) {
      gLine += c;
    }
  }

  // Auto-connect after scan finds peer, or periodic reconnect after drops.
  if (gHavePeer && !gBleConnected && !gConnectBusy && gAutoReconnect) {
    const bool scanTriggered = (gState == ST_CONNECTING);
    const bool due =
        scanTriggered ||
        (millis() - gLastReconnectAttemptMs) >= RECONNECT_INTERVAL_MS;
    if (due) {
      gLastReconnectAttemptMs = millis();
      bleConnectAndSubscribe();
    }
  }

  cpad_val_packet_t pkt;
  while (xQueueReceive(gPktQueue, &pkt, 0) == pdTRUE) {
    if (pkt.msg_type == CPAD_VAL_MSG_RELEASE_ALL ||
        pkt.msg_type == CPAD_VAL_MSG_KEYBOARD_REPORT) {
      applyKeyboardPacket(pkt, "ble");
    }
  }

  if (gBleConnected && gLastHeartbeatMs != 0 &&
      (millis() - gLastHeartbeatMs) > HEARTBEAT_TIMEOUT_MS) {
    Serial.println("[ble] heartbeat timeout");
    usbReleaseAll("heartbeat-timeout");
    if (gClient && gClient->isConnected()) gClient->disconnect();
    setState(ST_ERROR);
    gLastHeartbeatMs = 0;
    gLastReconnectAttemptMs = millis(); // backoff then auto-reconnect
  }
}
