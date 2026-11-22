import { describe, expect, it } from "vitest";
import { Decoder, encode, FrameError, FrameKind, MAX_FRAME } from "../src/frame.js";

function bytes(str: string): Uint8Array {
  return new TextEncoder().encode(str);
}

describe("encode", () => {
  it("round-trips a control frame", () => {
    const payload = bytes('{"type":"ok"}');
    const framed = encode(FrameKind.Control, payload);

    expect(framed[0]).toBe(0);
    const view = new DataView(framed.buffer, framed.byteOffset, framed.byteLength);
    expect(view.getUint32(1, false)).toBe(payload.length);
    expect(framed.slice(5)).toEqual(payload);
  });

  it("round-trips an output frame", () => {
    const payload = bytes("hello world");
    const framed = encode(FrameKind.Output, payload);
    expect(framed[0]).toBe(1);
  });

  it("round-trips an input frame", () => {
    const payload = bytes("ls -la\n");
    const framed = encode(FrameKind.Input, payload);
    expect(framed[0]).toBe(2);
  });

  it("round-trips an empty payload", () => {
    const framed = encode(FrameKind.Control, new Uint8Array(0));
    expect(framed.length).toBe(5);
  });

  it("throws on an unknown kind", () => {
    expect(() => encode(3 as never, bytes("x"))).toThrow(FrameError);
  });

  it("throws on an oversized payload", () => {
    const oversized = new Uint8Array(MAX_FRAME + 1);
    expect(() => encode(FrameKind.Output, oversized)).toThrow(FrameError);
  });

  it("accepts a payload at exactly MAX_FRAME", () => {
    const maxPayload = new Uint8Array(MAX_FRAME);
    expect(() => encode(FrameKind.Output, maxPayload)).not.toThrow();
  });
});

describe("Decoder", () => {
  it("decodes a single frame pushed whole", () => {
    const payload = bytes('{"type":"ok"}');
    const framed = encode(FrameKind.Control, payload);

    const decoder = new Decoder();
    decoder.push(framed);
    const frame = decoder.pop();

    expect(frame).toBeDefined();
    expect(frame?.kind).toBe(0);
    expect(frame?.payload).toEqual(payload);
    expect(decoder.pop()).toBeUndefined();
  });

  it("decodes a frame split across many small pushes", () => {
    const payload = bytes("the quick brown fox jumps over the lazy dog");
    const framed = encode(FrameKind.Output, payload);

    const decoder = new Decoder();
    for (const byte of framed) {
      expect(decoder.pop()).toBeUndefined();
      decoder.push(new Uint8Array([byte]));
    }

    const frame = decoder.pop();
    expect(frame?.kind).toBe(1);
    expect(frame?.payload).toEqual(payload);
  });

  it("decodes multiple concatenated frames in one push", () => {
    const a = encode(FrameKind.Control, bytes("one"));
    const b = encode(FrameKind.Input, bytes("two"));
    const c = encode(FrameKind.Output, bytes("three"));

    const merged = new Uint8Array(a.length + b.length + c.length);
    merged.set(a, 0);
    merged.set(b, a.length);
    merged.set(c, a.length + b.length);

    const decoder = new Decoder();
    decoder.push(merged);

    expect(decoder.pop()).toEqual({ kind: 0, payload: bytes("one") });
    expect(decoder.pop()).toEqual({ kind: 2, payload: bytes("two") });
    expect(decoder.pop()).toEqual({ kind: 1, payload: bytes("three") });
    expect(decoder.pop()).toBeUndefined();
  });

  it("throws as soon as an unknown kind byte is seen", () => {
    const decoder = new Decoder();
    const bad = new Uint8Array(5);
    bad[0] = 9; // unknown kind
    expect(() => decoder.push(bad)).toThrow(FrameError);
  });

  it("throws as soon as an oversized length is seen, before the body arrives", () => {
    const decoder = new Decoder();
    const header = new Uint8Array(5);
    header[0] = 1;
    const view = new DataView(header.buffer);
    view.setUint32(1, MAX_FRAME + 1, false);

    expect(() => decoder.push(header)).toThrow(FrameError);
  });

  it("returns undefined while waiting for more bytes", () => {
    const decoder = new Decoder();
    decoder.push(new Uint8Array([0, 0, 0]));
    expect(decoder.pop()).toBeUndefined();
  });
});
