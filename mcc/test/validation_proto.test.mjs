/**
 * Host-side encode/decode for Cyberpad validation protocol v0.
 * Mirrors firmware/common/validation_protocol.h
 */
import assert from "node:assert/strict";
import { test } from "node:test";

const VERSION = 0;
const SIZE = 12;
const MSG = {
  HELLO: 0x01,
  KEYBOARD_REPORT: 0x02,
  RELEASE_ALL: 0x03,
  HEARTBEAT: 0x04,
  LIGHTS: 0x05,
};

function checksum(bytes11) {
  let x = 0;
  for (let i = 0; i < 11; i++) x ^= bytes11[i];
  return x & 0xff;
}

function encode({ msgType, seq, modifiers = 0, keys = [0, 0, 0, 0, 0, 0] }) {
  const buf = Buffer.alloc(SIZE);
  buf.writeUInt8(VERSION, 0);
  buf.writeUInt8(msgType, 1);
  buf.writeUInt16LE(seq & 0xffff, 2);
  buf.writeUInt8(modifiers & 0xff, 4);
  for (let i = 0; i < 6; i++) buf.writeUInt8((keys[i] || 0) & 0xff, 5 + i);
  buf.writeUInt8(checksum(buf.subarray(0, 11)), 11);
  return buf;
}

function decode(buf) {
  if (!Buffer.isBuffer(buf) || buf.length !== SIZE) throw new Error("bad len");
  if (buf.readUInt8(0) !== VERSION) throw new Error("bad version");
  const msgType = buf.readUInt8(1);
  if (![MSG.HELLO, MSG.KEYBOARD_REPORT, MSG.RELEASE_ALL, MSG.HEARTBEAT, MSG.LIGHTS].includes(msgType)) {
    throw new Error("bad type");
  }
  const expect = checksum(buf.subarray(0, 11));
  if (buf.readUInt8(11) !== expect) throw new Error("bad checksum");
  return {
    version: VERSION,
    msgType,
    seq: buf.readUInt16LE(2),
    modifiers: buf.readUInt8(4),
    keys: [...buf.subarray(5, 11)],
  };
}

test("encode/decode KEYBOARD_REPORT A", () => {
  const pkt = encode({
    msgType: MSG.KEYBOARD_REPORT,
    seq: 7,
    modifiers: 0,
    keys: [0x04, 0, 0, 0, 0, 0],
  });
  assert.equal(pkt.length, 12);
  const d = decode(pkt);
  assert.equal(d.seq, 7);
  assert.deepEqual(d.keys, [0x04, 0, 0, 0, 0, 0]);
});

test("reject unknown version", () => {
  const pkt = encode({ msgType: MSG.HELLO, seq: 1 });
  pkt.writeUInt8(1, 0);
  pkt.writeUInt8(checksum(pkt.subarray(0, 11)), 11);
  assert.throws(() => decode(pkt), /version/);
});

test("reject unknown type", () => {
  const pkt = encode({ msgType: MSG.HELLO, seq: 1 });
  pkt.writeUInt8(0x99, 1);
  pkt.writeUInt8(checksum(pkt.subarray(0, 11)), 11);
  assert.throws(() => decode(pkt), /type/);
});

test("reject bad checksum", () => {
  const pkt = encode({ msgType: MSG.HEARTBEAT, seq: 2 });
  pkt.writeUInt8(pkt.readUInt8(11) ^ 0xff, 11);
  assert.throws(() => decode(pkt), /checksum/);
});

test("reject wrong length", () => {
  assert.throws(() => decode(Buffer.alloc(11)), /len/);
});

test("RELEASE_ALL and empty report round-trip", () => {
  const a = decode(encode({ msgType: MSG.RELEASE_ALL, seq: 9 }));
  assert.equal(a.msgType, MSG.RELEASE_ALL);
  const b = decode(
    encode({ msgType: MSG.KEYBOARD_REPORT, seq: 10, keys: [0, 0, 0, 0, 0, 0] }),
  );
  assert.deepEqual(b.keys, [0, 0, 0, 0, 0, 0]);
});

test("LIGHTS modifiers on/off", () => {
  const on = decode(encode({ msgType: MSG.LIGHTS, seq: 3, modifiers: 1 }));
  assert.equal(on.msgType, MSG.LIGHTS);
  assert.equal(on.modifiers, 1);
  const off = decode(encode({ msgType: MSG.LIGHTS, seq: 4, modifiers: 0 }));
  assert.equal(off.modifiers, 0);
});
