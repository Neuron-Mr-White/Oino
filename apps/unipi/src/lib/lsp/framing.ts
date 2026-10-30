/**
 * LSP `Content-Length` framing (app spec §3.12, WP-A21): the wire is
 * header-framed JSON — the tauri `openLsp` impl encodes outgoing JSON-RPC
 * messages to framed bytes and incrementally reassembles incoming byte
 * chunks back to whole message strings. The mock side never sees this —
 * it hands over parsed JSON directly.
 */

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** `{"jsonrpc":"2.0",…}` → `Content-Length: N\r\n\r\n<body>`. */
export function encodeMessage(body: string): Uint8Array {
  const payload = encoder.encode(body);
  const head = encoder.encode(`Content-Length: ${payload.length}\r\n\r\n`);
  const out = new Uint8Array(head.length + payload.length);
  out.set(head, 0);
  out.set(payload, head.length);
  return out;
}

/**
 * Incremental `Content-Length` parser. `push` returns every complete
 * message body (as a string — the JSON stays for the caller to parse,
 * so transport subscribers get exactly the server's bytes). Chunks can
 * split anywhere — mid-header, mid-body, several messages per chunk.
 */
export class LspFrameParser {
  private buffer: Uint8Array = new Uint8Array(0);
  private contentLength: number | undefined;

  push(chunk: Uint8Array): string[] {
    if (this.buffer.length === 0) {
      this.buffer = chunk;
    } else {
      const next = new Uint8Array(this.buffer.length + chunk.length);
      next.set(this.buffer, 0);
      next.set(chunk, this.buffer.length);
      this.buffer = next;
    }
    const out: string[] = [];
    for (;;) {
      if (this.contentLength === undefined) {
        const sep = findSequence(this.buffer, SEP);
        if (sep < 0) return out;
        const headerText = decoder.decode(this.buffer.subarray(0, sep));
        const len = contentLength(headerText);
        if (len === undefined) {
          // Not an LSP header — drop one byte and keep looking rather
          // than desyncing the whole stream.
          this.buffer = this.buffer.subarray(1);
          continue;
        }
        this.contentLength = len;
        this.buffer = this.buffer.subarray(sep + SEP.length);
      }
      if (this.buffer.length < this.contentLength) return out;
      out.push(decoder.decode(this.buffer.subarray(0, this.contentLength)));
      this.buffer = this.buffer.subarray(this.contentLength);
      this.contentLength = undefined;
    }
  }
}

const SEP = encoder.encode('\r\n\r\n');

function findSequence(haystack: Uint8Array, needle: Uint8Array): number {
  outer: for (let i = 0; i + needle.length <= haystack.length; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j]) continue outer;
    }
    return i;
  }
  return -1;
}

function contentLength(header: string): number | undefined {
  for (const line of header.split('\r\n')) {
    const colon = line.indexOf(':');
    if (colon < 0) continue;
    if (line.slice(0, colon).trim().toLowerCase() === 'content-length') {
      const len = Number(line.slice(colon + 1).trim());
      return Number.isFinite(len) && len >= 0 ? len : undefined;
    }
  }
  return undefined;
}
