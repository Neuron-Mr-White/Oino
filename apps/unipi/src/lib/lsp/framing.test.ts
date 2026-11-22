import { describe, expect, it } from 'vitest';
import { LspFrameParser, encodeMessage } from './framing';

const enc = new TextEncoder();
const dec = new TextDecoder();

const msg = (body: string) => `Content-Length: ${enc.encode(body).length}\r\n\r\n${body}`;

describe('encodeMessage', () => {
  it('frames with Content-Length headers', () => {
    const bytes = encodeMessage('{"a":1}');
    expect(dec.decode(bytes)).toBe('Content-Length: 7\r\n\r\n{"a":1}');
  });
});

describe('LspFrameParser', () => {
  it('parses one whole message', () => {
    const p = new LspFrameParser();
    expect(p.push(enc.encode(msg('{"id":1,"result":{}}')))).toEqual(['{"id":1,"result":{}}']);
  });

  it('joins a message split mid-header and mid-body', () => {
    const body = '{"jsonrpc":"2.0","result":{"capabilities":{}}}';
    const framed = enc.encode(msg(body));
    const p = new LspFrameParser();
    expect(p.push(framed.subarray(0, 10))).toEqual([]);
    expect(p.push(framed.subarray(10, 30))).toEqual([]);
    expect(p.push(framed.subarray(30))).toEqual([body]);
  });

  it('yields several messages from one chunk', () => {
    const p = new LspFrameParser();
    const two = enc.encode(msg('{"a":1}') + msg('{"b":2}'));
    expect(p.push(two)).toEqual(['{"a":1}', '{"b":2}']);
  });

  it('handles a chunk ending exactly at the header boundary', () => {
    const body = '{"m":true}';
    const framed = enc.encode(msg(body));
    const headerEnd = framed.findIndex((_, i) => framed[i] === 13 && framed[i + 1] === 10 && framed[i + 2] === 13 && framed[i + 3] === 10) + 4;
    const p = new LspFrameParser();
    expect(p.push(framed.subarray(0, headerEnd))).toEqual([]);
    expect(p.push(framed.subarray(headerEnd))).toEqual([body]);
  });

  it('keeps a byte-level stream in sync after unrelated bytes', () => {
    const p = new LspFrameParser();
    const out = p.push(enc.encode(`noise${msg('{"ok":true}')}`));
    expect(out).toEqual(['{"ok":true}']);
  });
});
