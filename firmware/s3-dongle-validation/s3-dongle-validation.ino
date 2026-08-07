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

#define FW_VERSION "s3-dongle-validation 0.5.3 protocol-v0.3"
#define DEVICE_USB_NAME "Cyberpad S3 Validation Dongle"
#define HEARTBEAT_TIMEOUT_MS 5000
#define HID_TEST_HOLD_MS 80
#define SCAN_WINDOW_MS 12000
#define RECONNECT_INTERVAL_MS 2500
// NimBLE defaults to a 30 s synchronous attempt plus two internal retries.
// CDC commands run in loop(), so bound the blackout below the 1.5 s host
// verifier window and let the outer reconnect cadence own retries.
#define BLE_CONNECT_TIMEOUT_MS 500
#define CDC_LINE_MAX 800
#define BANK_COUNT 5
#define HYBRID_SLOTS_BYTES 487
#define HYBRID_SLOTS_B64_BYTES 652
/* PoC-only known Cyberpad address (documented). Prefer UUID scan when possible. */
#define CPAD_VAL_PEER_ADDR "20:6E:F1:11:5F:36"
#define CYBERDECK_SERVICE_UUID "c0de0001-3d17-4a00-8000-00805f9b34fb"
#define CYBERDECK_SLOTS_UUID   "c0de0002-3d17-4a00-8000-00805f9b34fb"
#define CYBERDECK_MACRO_EVT_UUID "c0de0003-3d17-4a00-8000-00805f9b34fb"
#define CYBERDECK_INFO_UUID    "c0de0004-3d17-4a00-8000-00805f9b34fb"
#define CYBERDECK_BANK_SEL_UUID "c0de0005-3d17-4a00-8000-00805f9b34fb"

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

struct MacroEventWire {
  uint8_t bank;
  uint8_t preset;
  uint8_t action;
};

struct ValidationQueueItem {
  cpad_val_packet_t packet;
  bool fromPeer;
};

struct ScanResultWire {
  char address[18];
  char name[32];
  int8_t rssi;
  uint8_t addressType;
  bool validationUuidHit;
  bool cyberdeckUuidHit;
};

struct AsyncDiagnostics {
  uint32_t packetRejectsPending;
  uint32_t packetQueueDropsPending;
  uint32_t macroQueueDropsPending;
  uint32_t macroQueueDropsTotal;
  int lastPacketRejectRc;
};

static BridgeState gState = ST_BOOT;
static QueueHandle_t gPktQueue = nullptr;
static QueueHandle_t gMacroQueue = nullptr;
static QueueHandle_t gDisconnectQueue = nullptr;
static QueueHandle_t gBankQueue = nullptr;
static QueueHandle_t gScanQueue = nullptr;
static portMUX_TYPE gDiagnosticsMux = portMUX_INITIALIZER_UNLOCKED;
static AsyncDiagnostics gDiagnostics = {};
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
static NimBLERemoteCharacteristic *gMacroEvtChar = nullptr;
static NimBLERemoteCharacteristic *gInfoChar = nullptr;
static NimBLERemoteCharacteristic *gBankSelChar = nullptr;
static NimBLEAddress gPeerAddr;
static bool gHavePeer = false;
static bool gSlotsReady = false;
static bool gMacroReady = false;
static uint8_t gSelectedBank = 0;
static String gPadInfo;
static bool gLightsEnabled = true; // mirrors pad B3 indicator toggle

// Unsolicited packet chatter shares the single TinyUSB CDC stream with command
// replies. Once the pad is paired it notifies every 2 s, and that steady spam
// overruns the CDC TX buffer: replies come back truncated or spliced mid-line
// (e.g. "[ble] HEARTBEAT ses3-dongle-validation 0.5.1 ..."). When the casualty
// is a base64 bank page the host decodes shifted slot records and reports
// "slot label has nonzero bytes after its NUL terminator", which is what broke
// MCC sync. Measured at ~1 corrupted command in 5 with the pad connected.
//
// The host already skips whole async lines, so line interleaving was never the
// problem -- the truncation is. Default this off so the CDC channel stays clean
// for protocol traffic; `log on` restores it for humans debugging by hand.
// This was never caught because the CDC release gate only ever ran with the pad
// offline, where no heartbeats exist.
static bool gAsyncChatter = false;
static String gLine;

// ── Chain-safe long read for the 487-byte Slots page ────────────────────────
// NimBLE-Arduino's NimBLERemoteValueAttribute::onReadCB flattens ATT read
// chunks with `append(attr->om->om_data, OS_MBUF_PKTLEN(attr->om))`:
// PKTLEN counts the WHOLE mbuf chain, but om_data is only the FIRST link, so
// any response larger than one MSYS block (~275 data bytes with
// MSYS_2_BLOCK_SIZE=320) copies past the first link into adjacent pool memory.
// Result: bytes 0..274 arrive intact, everything after is stale pool contents
// -- old packets, old label fragments -- varying read to read. That is the
// deterministic slot-10 / byte-275 bank-page corruption that broke MCC sync.
// readValue() cannot be used for any value that can exceed one mbuf; this
// path reads the page with ble_gattc_read_long and walks the chain with
// os_mbuf_copydata like the library should have.
static uint8_t gPageBuf[HYBRID_SLOTS_BYTES];
static volatile uint16_t gPageLen = 0;
static volatile int gPageRc = -1;
static SemaphoreHandle_t gPageSem = nullptr;

static int slotsPageReadCB(uint16_t connHandle, const ble_gatt_error *error,
                           ble_gatt_attr *attr, void *arg) {
  (void)connHandle;
  (void)arg;
  if (error->status == 0 && attr) {
    const uint16_t len = OS_MBUF_PKTLEN(attr->om);
    if (gPageLen + len > sizeof(gPageBuf)) {
      gPageRc = BLE_ATT_ERR_INVALID_ATTR_VALUE_LEN;
      xSemaphoreGive(gPageSem);
      return BLE_ATT_ERR_INVALID_ATTR_VALUE_LEN;
    }
    os_mbuf_copydata(attr->om, 0, len, gPageBuf + gPageLen);
    gPageLen = gPageLen + len;
    return 0; // ask the stack for the next chunk
  }
  gPageRc = (error->status == BLE_HS_EDONE) ? 0 : error->status;
  xSemaphoreGive(gPageSem);
  return error->status;
}

// Returns 0 and fills gPageBuf/gPageLen on success, nonzero otherwise.
static int readSlotsPageChainSafe() {
  if (!gClient || !gClient->isConnected() || !gSlotsChar || !gPageSem) {
    return BLE_HS_ENOTCONN;
  }
  gPageLen = 0;
  gPageRc = -1;
  (void)xSemaphoreTake(gPageSem, 0); // drain any stale give
  int rc = ble_gattc_read_long(gClient->getConnHandle(), gSlotsChar->getHandle(),
                               0, slotsPageReadCB, nullptr);
  if (rc != 0) return rc;
  if (xSemaphoreTake(gPageSem, pdMS_TO_TICKS(3000)) != pdTRUE) {
    return BLE_HS_ETIMEOUT;
  }
  return gPageRc;
}

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

static void notePacketReject(int rc) {
  portENTER_CRITICAL(&gDiagnosticsMux);
  gDiagnostics.packetRejectsPending++;
  gDiagnostics.lastPacketRejectRc = rc;
  portEXIT_CRITICAL(&gDiagnosticsMux);
}

static void notePacketQueueDrop() {
  portENTER_CRITICAL(&gDiagnosticsMux);
  gDiagnostics.packetQueueDropsPending++;
  portEXIT_CRITICAL(&gDiagnosticsMux);
}

static void noteMacroQueueDrop() {
  portENTER_CRITICAL(&gDiagnosticsMux);
  gDiagnostics.macroQueueDropsPending++;
  gDiagnostics.macroQueueDropsTotal++;
  portEXIT_CRITICAL(&gDiagnosticsMux);
}

static uint32_t macroQueueDropsTotal() {
  portENTER_CRITICAL(&gDiagnosticsMux);
  const uint32_t total = gDiagnostics.macroQueueDropsTotal;
  portEXIT_CRITICAL(&gDiagnosticsMux);
  return total;
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

static void queueReleaseAll() {
  if (!gPktQueue) return;
  xQueueReset(gPktQueue);
  ValidationQueueItem item = {};
  cpad_val_encode(&item.packet, CPAD_VAL_MSG_RELEASE_ALL, 0, 0, nullptr);
  item.fromPeer = false;
  (void)xQueueSend(gPktQueue, &item, 0);
}

static void enqueueValidated(const uint8_t *data, size_t len) {
  if (!gPktQueue) return;
  cpad_val_packet_t pkt;
  int rc = cpad_val_decode(data, len, &pkt);
  if (rc != 0) {
    notePacketReject(rc);
    // Safety: malformed after a held key could strand modifiers.
    queueReleaseAll();
    return;
  }
  const ValidationQueueItem item = {pkt, true};
  if (xQueueSend(gPktQueue, &item, 0) != pdTRUE) {
    notePacketQueueDrop();
    queueReleaseAll();
  }
}

static void notifyCB(NimBLERemoteCharacteristic * /*c*/, uint8_t *data, size_t len,
                     bool /*isNotify*/) {
  // Keep short — queue for loop().
  enqueueValidated(data, len);
}

static void bankSelNotifyCB(NimBLERemoteCharacteristic * /*c*/, uint8_t *data,
                            size_t len, bool /*isNotify*/) {
  if (!gBankQueue || len != 1 || data[0] >= BANK_COUNT) return;
  const uint8_t bank = data[0];
  (void)xQueueOverwrite(gBankQueue, &bank);
}

static void macroEvtNotifyCB(NimBLERemoteCharacteristic * /*c*/, uint8_t *data,
                             size_t len, bool /*isNotify*/) {
  if (len != 3 || data[0] >= BANK_COUNT || data[1] >= 6 || data[2] >= 3) return;
  const MacroEventWire event = {data[0], data[1], data[2]};
  if (gMacroQueue && xQueueSend(gMacroQueue, &event, 0) != pdTRUE) {
    noteMacroQueueDrop();
  }
}

class ClientCallbacks : public NimBLEClientCallbacks {
  void onConnect(NimBLEClient * /*pClient*/) override {}
  void onDisconnect(NimBLEClient * /*pClient*/, int reason) override {
    if (gDisconnectQueue) (void)xQueueOverwrite(gDisconnectQueue, &reason);
  }
};

static ClientCallbacks gClientCb;

class ScanCallbacks : public NimBLEScanCallbacks {
  void onResult(const NimBLEAdvertisedDevice *adv) override {
    const bool validationUuidHit =
        adv->isAdvertisingService(NimBLEUUID(CPAD_VAL_SERVICE_UUID));
    const bool cyberdeckUuidHit =
        adv->isAdvertisingService(NimBLEUUID(CYBERDECK_SERVICE_UUID));
    // Name fallback: UUID may only appear after active-scan response. Accept
    // both the validation identity and the protocol-contract identity.
    const bool nameHit =
        adv->haveName() &&
        (adv->getName().find("Cyberpad Val") != std::string::npos ||
         adv->getName().find("Cyberdeck Pad") != std::string::npos);
    if (!validationUuidHit && !cyberdeckUuidHit && !nameHit) return;
    if (!gScanQueue) return;
    ScanResultWire result = {};
    const std::string address = adv->getAddress().toString();
    snprintf(result.address, sizeof(result.address), "%s", address.c_str());
    if (adv->haveName()) {
      snprintf(result.name, sizeof(result.name), "%s", adv->getName().c_str());
    }
    result.rssi = adv->getRSSI();
    result.addressType = adv->getAddress().getType();
    result.validationUuidHit = validationUuidHit;
    result.cyberdeckUuidHit = cyberdeckUuidHit;
    (void)xQueueOverwrite(gScanQueue, &result);
    NimBLEDevice::getScan()->stop();
  }
};

static ScanCallbacks gScanCb;

static void clearRemoteState() {
  gNotifyChar = nullptr;
  gSlotsChar = nullptr;
  gMacroEvtChar = nullptr;
  gInfoChar = nullptr;
  gBankSelChar = nullptr;
  gSlotsReady = false;
  gMacroReady = false;
  gPadInfo = "";
}

static void processAsyncState() {
  int reason = 0;
  if (gDisconnectQueue && xQueueReceive(gDisconnectQueue, &reason, 0) == pdTRUE) {
    gBleConnected = false;
    clearRemoteState();
    if (gMacroQueue) xQueueReset(gMacroQueue);
    if (gPktQueue) xQueueReset(gPktQueue);
    if (gBankQueue) xQueueReset(gBankQueue);
    gConnectBusy = false;
    gLastReconnectAttemptMs = millis(); // back off before reconnecting
    setState(ST_DISCONNECTED);
    Serial.printf("[ble] disconnected reason=%d\n", reason);
    queueReleaseAll();
  }

  ScanResultWire scan = {};
  if (gScanQueue && xQueueReceive(gScanQueue, &scan, 0) == pdTRUE) {
    gPeerAddr = NimBLEAddress(std::string(scan.address), scan.addressType);
    gHavePeer = true;
    gBleWantScan = false;
    setState(ST_CONNECTING);
    Serial.printf("[ble] found peer %s rssi=%d val_uuid=%d cyberdeck_uuid=%d name=%s\n",
                  scan.address, (int)scan.rssi, (int)scan.validationUuidHit,
                  (int)scan.cyberdeckUuidHit, scan.name);
  }

  uint8_t bank = 0;
  if (gBankQueue && xQueueReceive(gBankQueue, &bank, 0) == pdTRUE) {
    gSelectedBank = bank;
    Serial.printf("[bank] selected %u\n", (unsigned)bank);
  }

  AsyncDiagnostics pending = {};
  portENTER_CRITICAL(&gDiagnosticsMux);
  pending.packetRejectsPending = gDiagnostics.packetRejectsPending;
  pending.packetQueueDropsPending = gDiagnostics.packetQueueDropsPending;
  pending.macroQueueDropsPending = gDiagnostics.macroQueueDropsPending;
  pending.lastPacketRejectRc = gDiagnostics.lastPacketRejectRc;
  gDiagnostics.packetRejectsPending = 0;
  gDiagnostics.packetQueueDropsPending = 0;
  gDiagnostics.macroQueueDropsPending = 0;
  portEXIT_CRITICAL(&gDiagnosticsMux);
  if (pending.packetRejectsPending) {
    Serial.printf("[ble] rejected %u packet(s), last rc=%d — release-all\n",
                  (unsigned)pending.packetRejectsPending,
                  pending.lastPacketRejectRc);
  }
  if (pending.packetQueueDropsPending) {
    Serial.printf("[ble] packet queue overflow x%u — release-all\n",
                  (unsigned)pending.packetQueueDropsPending);
  }
  if (pending.macroQueueDropsPending) {
    Serial.printf("[ble] macro queue dropped %u event(s), total=%u\n",
                  (unsigned)pending.macroQueueDropsPending,
                  (unsigned)macroQueueDropsTotal());
  }
}

static bool padInfoSupportsV03(const String &info) {
  static const char prefix[] = "Cyberdeck Pad Hybrid v";
  if (!info.startsWith(prefix)) return false;
  unsigned major = 0;
  unsigned minor = 0;
  unsigned patch = 0;
  char trailing = '\0';
  if (sscanf(info.c_str() + sizeof(prefix) - 1, "%u.%u.%u%c",
             &major, &minor, &patch, &trailing) != 3) return false;
  char canonical[34]; // 3 x uint32 decimal + two dots + NUL.
  const int canonicalLen =
      snprintf(canonical, sizeof(canonical), "%u.%u.%u", major, minor, patch);
  if (canonicalLen < 0 || size_t(canonicalLen) >= sizeof(canonical) ||
      strcmp(info.c_str() + sizeof(prefix) - 1, canonical) != 0) {
    return false;
  }
  // The page/event wire format is defined only for protocol 0.3.x. Fail closed
  // on later versions until their compatibility is explicitly reviewed.
  return major == 0 && minor == 3;
}

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
    gClient->setConnectTimeout(BLE_CONNECT_TIMEOUT_MS);
    gClient->setConnectRetries(0);
  }
  // Manual/duplicate connect requests are idempotent. Disconnecting here and
  // immediately reconnecting lets the old disconnect callback arrive after a
  // new success and clear the new generation's remote-characteristic state.
  if (gClient->isConnected()) {
    gBleConnected = true;
    gConnectBusy = false;
    Serial.println("[ble] already connected");
    return true;
  }

  if (!gClient->connect(gPeerAddr)) {
    Serial.println("[ble] connect failed");
    setState(ST_ERROR);
    gConnectBusy = false;
    gLastReconnectAttemptMs = millis();
    return false;
  }

  gBleConnected = true;
  setState(ST_DISCOVERING);
  Serial.println("[ble] connected");
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
  gMacroReady = false;
  gSlotsChar = nullptr;
  gMacroEvtChar = nullptr;
  gInfoChar = nullptr;
  gBankSelChar = nullptr;
  gPadInfo = "";
  gSelectedBank = 0;
  if (gBankQueue) xQueueReset(gBankQueue);
  NimBLERemoteService *hyb = gClient->getService(CYBERDECK_SERVICE_UUID);
  if (hyb) {
    gSlotsChar = hyb->getCharacteristic(CYBERDECK_SLOTS_UUID);
    gMacroEvtChar = hyb->getCharacteristic(CYBERDECK_MACRO_EVT_UUID);
    gInfoChar = hyb->getCharacteristic(CYBERDECK_INFO_UUID);
    gBankSelChar = hyb->getCharacteristic(CYBERDECK_BANK_SEL_UUID);

    if (gInfoChar && gInfoChar->canRead()) {
      NimBLEAttValue infoVal = gInfoChar->readValue();
      for (size_t i = 0; i < infoVal.size(); i++) gPadInfo += char(infoVal[i]);
    }
    const bool versionOk = padInfoSupportsV03(gPadInfo);
    const bool slotsCharsOk =
        gSlotsChar && gSlotsChar->canRead() &&
        (gSlotsChar->canWrite() || gSlotsChar->canWriteNoResponse()) &&
        gBankSelChar && gBankSelChar->canRead() &&
        (gBankSelChar->canWrite() || gBankSelChar->canWriteNoResponse()) &&
        gBankSelChar->canNotify();

    bool bankReadOk = false;
    bool bankNotifyOk = false;
    if (gBankSelChar && gBankSelChar->canRead()) {
      NimBLEAttValue bankVal = gBankSelChar->readValue();
      if (bankVal.size() == 1 && bankVal[0] < BANK_COUNT) {
        gSelectedBank = bankVal[0];
        bankReadOk = true;
      }
      if (bankReadOk && gBankSelChar->canNotify()) {
        bankNotifyOk = gBankSelChar->subscribe(true, bankSelNotifyCB);
      }
    }
    gSlotsReady = versionOk && slotsCharsOk && bankReadOk && bankNotifyOk;
    if (versionOk && gMacroEvtChar && gMacroEvtChar->canNotify()) {
      gMacroReady = gMacroEvtChar->subscribe(true, macroEvtNotifyCB);
    }
    Serial.printf(
        "[ble] pad_info='%s' slots_ready=%d macro_ready=%d bank=%u "
        "bank_read=%d bank_notify=%d\n",
        gPadInfo.c_str(), (int)gSlotsReady, (int)gMacroReady,
        (unsigned)gSelectedBank, (int)bankReadOk, (int)bankNotifyOk);
    if (!versionOk) Serial.println("[ble] protocol mismatch — slots sync disabled");
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
  Serial.println("[ble] scan start (validation/Cyberdeck UUID + known names)");
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
  Serial.println(F("  slots read <bank> | slots write <bank> <b64> | macro next | pad info"));
  Serial.println(F("  log on | log off   (async [ble] chatter; off by default"));
  Serial.println(F("                      so it cannot corrupt command replies)"));
}

static bool parseBankArg(const String &arg, uint8_t &bank) {
  String value = arg;
  value.trim();
  if (value.length() != 1 || value[0] < '0' || value[0] >= char('0' + BANK_COUNT)) {
    return false;
  }
  bank = uint8_t(value[0] - '0');
  return true;
}

static bool selectBank(uint8_t bank) {
  if (!gBleConnected || !gSlotsReady || !gBankSelChar || bank >= BANK_COUNT) {
    return false;
  }
  if (!gBankSelChar->writeValue(&bank, 1, true)) return false;
  NimBLEAttValue selected = gBankSelChar->readValue();
  if (selected.size() != 1 || selected[0] != bank) return false;
  gSelectedBank = bank;
  return true;
}

static void cmdSlotsRead(uint8_t bank) {
  if (!gBleConnected || !gSlotsReady || !gSlotsChar || !selectBank(bank)) {
    Serial.println("ERR slots not ready");
    return;
  }
  const int readRc = readSlotsPageChainSafe();
  if (readRc != 0 || gPageLen != HYBRID_SLOTS_BYTES || gPageBuf[0] != bank) {
    Serial.printf("ERR slots page rc=%d len=%u bank=%d want=%u\n", readRc,
                  (unsigned)gPageLen, gPageLen ? int(gPageBuf[0]) : -1,
                  (unsigned)bank);
    return;
  }
  char b64[700];
  int n = cpad_b64_encode(gPageBuf, HYBRID_SLOTS_BYTES, b64, sizeof(b64));
  if (n < 0) {
    Serial.println("ERR b64 encode");
    return;
  }
  // One call keeps the machine-readable record indivisible from diagnostics.
  Serial.printf("SLOTS %u %s\n", (unsigned)bank, b64);
}

static void cmdSlotsWrite(uint8_t bank, const String &b64part) {
  if (!gBleConnected || !gSlotsReady || !gSlotsChar) {
    Serial.println("ERR slots not ready");
    return;
  }
  String b64 = b64part;
  b64.trim();
  if (b64.length() != HYBRID_SLOTS_B64_BYTES ||
      b64[HYBRID_SLOTS_B64_BYTES - 2] != '=' ||
      b64[HYBRID_SLOTS_B64_BYTES - 1] != '=') {
    Serial.println("ERR usage: slots write <bank> <base64-487>");
    return;
  }
  uint8_t raw[HYBRID_SLOTS_BYTES];
  int n = cpad_b64_decode(b64.c_str(), b64.length(), raw, sizeof(raw));
  if (n != HYBRID_SLOTS_BYTES) {
    Serial.printf("ERR b64 decode got=%d want=%d\n", n, HYBRID_SLOTS_BYTES);
    return;
  }
  if (raw[0] != bank) {
    Serial.printf("ERR payload bank=%u want=%u\n", (unsigned)raw[0],
                  (unsigned)bank);
    return;
  }
  // Malformed data must not have a BankSel side effect.
  if (!selectBank(bank)) {
    Serial.println("ERR slots not ready");
    return;
  }
  if (!gSlotsChar->writeValue(raw, HYBRID_SLOTS_BYTES, true)) {
    Serial.println("ERR slots ble write");
    return;
  }
  // Chain-safe read here too: library readValue() corrupts >1-mbuf values, and
  // a corrupt verify would reject good writes (or worse, pass bad ones).
  const int verifyRc = readSlotsPageChainSafe();
  if (verifyRc != 0 || gPageLen != HYBRID_SLOTS_BYTES ||
      memcmp(gPageBuf, raw, HYBRID_SLOTS_BYTES) != 0) {
    Serial.printf("ERR slots verify mismatch rc=%d len=%u\n", verifyRc,
                  (unsigned)gPageLen);
    return;
  }
  Serial.printf("OK bank=%u verified\n", (unsigned)bank);
}

static void cmdPadInfo() {
  if (!gBleConnected || !gInfoChar) {
    Serial.println("ERR pad info unavailable");
    return;
  }
  NimBLEAttValue val = gInfoChar->readValue();
  String response = "INFO ";
  if (!response.reserve(5 + val.size())) {
    Serial.println("ERR pad info allocation");
    return;
  }
  for (size_t i = 0; i < val.size(); i++) response += char(val[i]);
  // One call keeps the machine-readable record indivisible from diagnostics.
  Serial.println(response);
}

static void cmdMacroNext() {
  Serial.printf("BANK %u\n", (unsigned)gSelectedBank);
  MacroEventWire event;
  if (!gMacroQueue || xQueueReceive(gMacroQueue, &event, 0) != pdTRUE) {
    Serial.println("NONE");
    return;
  }
  Serial.printf("MACRO %u %u %u\n", (unsigned)event.bank,
                (unsigned)event.preset, (unsigned)event.action);
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
    const int split = rest.indexOf(' ');
    uint8_t bank = 0;
    if (split < 0 || !parseBankArg(rest.substring(0, split), bank)) {
      Serial.println("ERR usage: slots write <bank> <base64-487>");
      return;
    }
    String b64 = rest.substring(split + 1);
    b64.trim();
    cmdSlotsWrite(bank, b64);
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
        "macro_ready=%d bank=%u queue=%u macro_queue=%u macro_drops=%u "
        "transport=dongle protocol=v0.3\n",
        stateName(gState), (int)gBleConnected, (int)gHavePeer, (int)gAutoReconnect,
        (int)gSlotsReady, (int)gMacroReady, (unsigned)gSelectedBank,
        (unsigned)uxQueueMessagesWaiting(gPktQueue),
        (unsigned)uxQueueMessagesWaiting(gMacroQueue),
        (unsigned)macroQueueDropsTotal());
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
  } else if (cmd == "log on" || cmd == "log off") {
    gAsyncChatter = (cmd == "log on");
    Serial.printf("async chatter %s\n", gAsyncChatter ? "on" : "off");
  } else if (cmd == "peer show") {
    if (gHavePeer) Serial.printf("peer %s\n", gPeerAddr.toString().c_str());
    else Serial.println("peer (none)");
  } else if (cmd.startsWith("slots read ")) {
    uint8_t bank = 0;
    if (!parseBankArg(cmd.substring(11), bank)) {
      Serial.println("ERR usage: slots read <bank>");
    } else {
      cmdSlotsRead(bank);
    }
  } else if (cmd == "slots read") {
    Serial.println("ERR usage: slots read <bank>");
  } else if (cmd == "pad info") {
    cmdPadInfo();
  } else if (cmd == "macro next") {
    cmdMacroNext();
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

  gPktQueue = xQueueCreate(8, sizeof(ValidationQueueItem));
  gMacroQueue = xQueueCreate(16, sizeof(MacroEventWire));
  gDisconnectQueue = xQueueCreate(1, sizeof(int));
  gBankQueue = xQueueCreate(1, sizeof(uint8_t));
  gScanQueue = xQueueCreate(1, sizeof(ScanResultWire));
  gPageSem = xSemaphoreCreateBinary();
  if (!gPktQueue || !gMacroQueue || !gDisconnectQueue || !gBankQueue ||
      !gScanQueue || !gPageSem) {
    Serial.println("[fatal] queue allocation failed; restarting");
    delay(1000);
    ESP.restart();
    return;
  }
  memset(&gLastApplied, 0, sizeof(gLastApplied));

  // Central-only: do not advertise as peripheral (interferes with scanning).
  NimBLEDevice::init("");
  NimBLEDevice::setMTU(517);
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
  processAsyncState();
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
  // Apply callbacks only after any in-flight CDC command has returned. Remote
  // characteristic pointers are consequently owned by loop() alone.
  processAsyncState();

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
  processAsyncState();

  ValidationQueueItem item;
  while (xQueueReceive(gPktQueue, &item, 0) == pdTRUE) {
    const cpad_val_packet_t &pkt = item.packet;
    if (item.fromPeer) gLastHeartbeatMs = millis();
    if (pkt.msg_type == CPAD_VAL_MSG_HEARTBEAT ||
        pkt.msg_type == CPAD_VAL_MSG_HELLO) {
      if (gAsyncChatter) {
        Serial.printf("[ble] %s seq=%u\n",
                      pkt.msg_type == CPAD_VAL_MSG_HELLO ? "HELLO" : "HEARTBEAT",
                      (unsigned)pkt.seq);
      }
    } else if (pkt.msg_type == CPAD_VAL_MSG_LIGHTS) {
      gLightsEnabled = (pkt.modifiers != 0);
      if (!gLightsEnabled) connNeoOff();
      if (gAsyncChatter) {
        Serial.printf("[ble] LIGHTS %s seq=%u\n",
                      gLightsEnabled ? "on" : "off", (unsigned)pkt.seq);
      }
    } else if (pkt.msg_type == CPAD_VAL_MSG_RELEASE_ALL ||
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
