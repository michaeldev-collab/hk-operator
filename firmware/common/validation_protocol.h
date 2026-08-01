/**
 * Cyberpad C6↔S3 validation protocol v0 — experimental.
 * See docs/validation/c6-s3-validation-protocol.md
 */
#pragma once

#include <stdint.h>
#include <stddef.h>
#include <string.h>

#ifdef __cplusplus
extern "C" {
#endif

#define CPAD_VAL_PROTO_VERSION 0
#define CPAD_VAL_PACKET_SIZE   12

#define CPAD_VAL_MSG_HELLO            0x01
#define CPAD_VAL_MSG_KEYBOARD_REPORT  0x02
#define CPAD_VAL_MSG_RELEASE_ALL      0x03
#define CPAD_VAL_MSG_HEARTBEAT        0x04
/* modifiers: 1 = pad indicator lights on, 0 = off (NeoPixels follow). */
#define CPAD_VAL_MSG_LIGHTS           0x05

/* Stable experimental UUIDs — do not regenerate */
#define CPAD_VAL_SERVICE_UUID "c0de1001-3d17-4a00-8000-00805f9b34fb"
#define CPAD_VAL_NOTIFY_UUID  "c0de1002-3d17-4a00-8000-00805f9b34fb"

typedef struct __attribute__((packed)) {
  uint8_t  version;
  uint8_t  msg_type;
  uint16_t seq;          /* little-endian on the wire */
  uint8_t  modifiers;
  uint8_t  keys[6];
  uint8_t  checksum;
} cpad_val_packet_t;

#if defined(__cplusplus)
static_assert(sizeof(cpad_val_packet_t) == CPAD_VAL_PACKET_SIZE,
              "validation packet must be 12 bytes");
#else
_Static_assert(sizeof(cpad_val_packet_t) == CPAD_VAL_PACKET_SIZE,
               "validation packet must be 12 bytes");
#endif

static inline uint8_t cpad_val_checksum(const uint8_t *bytes11) {
  uint8_t x = 0;
  for (int i = 0; i < 11; i++) x ^= bytes11[i];
  return x;
}

static inline int cpad_val_msg_type_ok(uint8_t t) {
  return t == CPAD_VAL_MSG_HELLO || t == CPAD_VAL_MSG_KEYBOARD_REPORT ||
         t == CPAD_VAL_MSG_RELEASE_ALL || t == CPAD_VAL_MSG_HEARTBEAT ||
         t == CPAD_VAL_MSG_LIGHTS;
}

static inline void cpad_val_encode(cpad_val_packet_t *out, uint8_t msg_type,
                                   uint16_t seq, uint8_t modifiers,
                                   const uint8_t keys[6]) {
  memset(out, 0, sizeof(*out));
  out->version = CPAD_VAL_PROTO_VERSION;
  out->msg_type = msg_type;
  out->seq = seq; /* LE on little-endian MCUs; explicit LE pack also OK */
  out->modifiers = modifiers;
  if (keys) memcpy(out->keys, keys, 6);
  out->checksum = cpad_val_checksum((const uint8_t *)out);
}

/** Returns 0 on success, negative on reject. Never trust out on failure. */
static inline int cpad_val_decode(const uint8_t *buf, size_t len,
                                  cpad_val_packet_t *out) {
  if (!buf || !out || len != CPAD_VAL_PACKET_SIZE) return -1;
  cpad_val_packet_t tmp;
  memcpy(&tmp, buf, CPAD_VAL_PACKET_SIZE);
  if (tmp.version != CPAD_VAL_PROTO_VERSION) return -2;
  if (!cpad_val_msg_type_ok(tmp.msg_type)) return -3;
  uint8_t expect = cpad_val_checksum((const uint8_t *)&tmp);
  if (tmp.checksum != expect) return -4;
  *out = tmp;
  return 0;
}

#ifdef __cplusplus
}
#endif
