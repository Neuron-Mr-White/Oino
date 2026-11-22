/**
 * Frame encoding for `unipi/app/1`: `u8 kind · u32BE length · payload`.
 *
 * Kinds:
 * - 0: JSON control (UTF-8)
 * - 1: terminal output bytes
 * - 2: terminal input bytes
 */

export const FRAME_HEADER_LEN = 5;
export const MAX_FRAME = 1024 * 1024; // 1 MiB payload limit

export const FrameKind = {
  Control: 0,
  Output: 1,
  Input: 2,
} as const;

export type FrameKindValue = (typeof FrameKind)[keyof typeof FrameKind];

export class FrameError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "FrameError";
  }
}

function isValidKind(kind: number): kind is FrameKindValue {
  return kind === 0 || kind === 1 || kind === 2;
}

/** Encodes one frame: `[kind: u8][len: u32BE][payload]`. */
export function encode(kind: FrameKindValue, payload: Uint8Array): Uint8Array {
  if (!isValidKind(kind)) {
    throw new FrameError(`unknown frame kind: ${kind}`);
  }
  if (payload.length > MAX_FRAME) {
    throw new FrameError(
      `frame payload too large: ${payload.length} > ${MAX_FRAME}`,
    );
  }

  const out = new Uint8Array(FRAME_HEADER_LEN + payload.length);
  out[0] = kind;
  const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
  view.setUint32(1, payload.length, false);
  out.set(payload, FRAME_HEADER_LEN);
  return out;
}

export interface DecodedFrame {
  kind: FrameKindValue;
  payload: Uint8Array;
}

/**
 * Incremental frame decoder. Push arbitrary byte chunks with `push`,
 * then drain fully-received frames with `pop`. Throws `FrameError`
 * as soon as an invalid header (unknown kind, oversized length) is
 * seen, even if the full frame hasn't arrived yet.
 */
export class Decoder {
  private buffer: Uint8Array = new Uint8Array(0);

  push(chunk: Uint8Array): void {
    const merged = new Uint8Array(this.buffer.length + chunk.length);
    merged.set(this.buffer, 0);
    merged.set(chunk, this.buffer.length);
    this.buffer = merged;
    this.validateHeaderIfPresent();
  }

  /** Returns the next fully-buffered frame, or `undefined` if none yet. */
  pop(): DecodedFrame | undefined {
    if (this.buffer.length < FRAME_HEADER_LEN) {
      return undefined;
    }

    const kind = this.buffer[0]!;
    const view = new DataView(
      this.buffer.buffer,
      this.buffer.byteOffset,
      this.buffer.byteLength,
    );
    const len = view.getUint32(1, false);

    this.assertValidHeader(kind, len);

    const total = FRAME_HEADER_LEN + len;
    if (this.buffer.length < total) {
      return undefined;
    }

    const payload = this.buffer.slice(FRAME_HEADER_LEN, total);
    this.buffer = this.buffer.slice(total);
    return { kind: kind as FrameKindValue, payload };
  }

  private validateHeaderIfPresent(): void {
    if (this.buffer.length < FRAME_HEADER_LEN) {
      return;
    }
    const kind = this.buffer[0]!;
    const view = new DataView(
      this.buffer.buffer,
      this.buffer.byteOffset,
      this.buffer.byteLength,
    );
    const len = view.getUint32(1, false);
    this.assertValidHeader(kind, len);
  }

  private assertValidHeader(kind: number, len: number): void {
    if (!isValidKind(kind)) {
      throw new FrameError(`unknown frame kind: ${kind}`);
    }
    if (len > MAX_FRAME) {
      throw new FrameError(`frame payload too large: ${len} > ${MAX_FRAME}`);
    }
  }
}
