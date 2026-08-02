/**
 * Minimal base64 encode/decode for CDC slot payloads (486 bytes → ~648 chars).
 */
#pragma once

#include <stddef.h>
#include <stdint.h>

static const char CPAD_B64_TAB[] =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

static inline int cpad_b64_index(char c) {
  if (c >= 'A' && c <= 'Z') return c - 'A';
  if (c >= 'a' && c <= 'z') return c - 'a' + 26;
  if (c >= '0' && c <= '9') return c - '0' + 52;
  if (c == '+') return 62;
  if (c == '/') return 63;
  return -1;
}

/** Returns encoded length excluding NUL, or -1 if outbuf too small. */
static inline int cpad_b64_encode(const uint8_t *in, size_t in_len, char *out,
                                  size_t out_cap) {
  size_t need = ((in_len + 2) / 3) * 4 + 1;
  if (out_cap < need) return -1;
  size_t o = 0;
  for (size_t i = 0; i < in_len; i += 3) {
    uint32_t n = ((uint32_t)in[i]) << 16;
    if (i + 1 < in_len) n |= ((uint32_t)in[i + 1]) << 8;
    if (i + 2 < in_len) n |= (uint32_t)in[i + 2];
    out[o++] = CPAD_B64_TAB[(n >> 18) & 63];
    out[o++] = CPAD_B64_TAB[(n >> 12) & 63];
    out[o++] = (i + 1 < in_len) ? CPAD_B64_TAB[(n >> 6) & 63] : '=';
    out[o++] = (i + 2 < in_len) ? CPAD_B64_TAB[n & 63] : '=';
  }
  out[o] = '\0';
  return (int)o;
}

/** Returns decoded byte count, or -1 on error. */
static inline int cpad_b64_decode(const char *in, size_t in_len, uint8_t *out,
                                  size_t out_cap) {
  if (in_len % 4 != 0) return -1;
  size_t o = 0;
  for (size_t i = 0; i < in_len; i += 4) {
    int a = cpad_b64_index(in[i]);
    int b = cpad_b64_index(in[i + 1]);
    int c = in[i + 2] == '=' ? 0 : cpad_b64_index(in[i + 2]);
    int d = in[i + 3] == '=' ? 0 : cpad_b64_index(in[i + 3]);
    if (a < 0 || b < 0 || (in[i + 2] != '=' && c < 0) ||
        (in[i + 3] != '=' && d < 0))
      return -1;
    uint32_t n = ((uint32_t)a << 18) | ((uint32_t)b << 12) | ((uint32_t)c << 6) |
                 (uint32_t)d;
    if (o >= out_cap) return -1;
    out[o++] = (uint8_t)((n >> 16) & 0xff);
    if (in[i + 2] != '=') {
      if (o >= out_cap) return -1;
      out[o++] = (uint8_t)((n >> 8) & 0xff);
    }
    if (in[i + 3] != '=') {
      if (o >= out_cap) return -1;
      out[o++] = (uint8_t)(n & 0xff);
    }
  }
  return (int)o;
}
