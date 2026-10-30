/**
 * Live terminal session: wraps `term_open`/`term_input`/`term_resize`/
 * `term_close` plus the two Tauri channels (DESIGN §5.4). Every event
 * is zod-parsed before it reaches the caller; a parse failure closes
 * the session with E_SCHEMA. `open()` uses `call()` so host `err`
 * envelopes and transport failures surface as `{ok:false, error}`
 * instead of throwing.
 */
import { invoke } from '@tauri-apps/api/core';
import { Channel } from '@tauri-apps/api/core';
import type { AttachTarget, AppError } from '@unipi/protocol';
import { TermEventSchema, TermOpenResultSchema, type TermEvent, type TermOpenResult } from './term';
import { parseTermEvent } from './term';
import { call, type Result } from './ipc';
import type { PathKind } from './schemas';
import type { Relay } from '../settings/schema';

export type TermCallbacks = {
  onData: (bytes: Uint8Array) => void;
  onEvent: (event: TermEvent) => void;
  /** Stream ended (closed) or a terminal write failed. */
  onEnded?: (error?: AppError) => void;
};

export type TermOpenArgs = {
  hostId: string;
  relay?: Relay;
  target: AttachTarget | null;
  cols: number;
  rows: number;
  sizePolicy: 'fit_phone' | 'keep_pc';
};

export type TermOpenOk = {
  termId: string;
  effective_size_policy: 'fit_phone' | 'keep_pc';
  path: PathKind;
};

export type TermOpenOutcome =
  | { ok: true; value: TermOpenOk }
  | { ok: false; error: AppError };

function relayArg(relay?: Relay) {
  return relay?.mode === 'custom' ? relay.url : 'n0';
}

export class TermSession {
  private termId: string | undefined;
  private readonly callbacks: TermCallbacks;
  private readonly args: TermOpenArgs;

  constructor(args: TermOpenArgs, callbacks: TermCallbacks) {
    this.args = args;
    this.callbacks = callbacks;
  }

  /** Opens the session; never throws. */
  async open(): Promise<TermOpenOutcome> {
    const on_data = new Channel<Uint8Array>();
    on_data.onmessage = (bytes) => this.callbacks.onData(new Uint8Array(bytes));
    const on_event = new Channel<unknown>();
    on_event.onmessage = (raw) => {
      const parsed = parseTermEvent(raw);
      if (!parsed.ok) {
        this.callbacks.onEvent({ type: 'closed', error: parsed.error });
        this.callbacks.onEnded?.(parsed.error);
        return;
      }
      this.callbacks.onEvent(parsed.event);
      if (parsed.event.type === 'closed') {
        this.callbacks.onEnded?.(parsed.event.error);
      }
    };
    const result: Result<TermOpenResult, AppError> = await call(
      'term_open',
      {
        hostId: this.args.hostId,
        relay: relayArg(this.args.relay),
        target: this.args.target,
        cols: this.args.cols,
        rows: this.args.rows,
        sizePolicy: this.args.sizePolicy,
        onData: on_data,
        onEvent: on_event,
      },
      TermOpenResultSchema,
    );
    if (!result.ok) {
      return { ok: false, error: result.error };
    }
    this.termId = result.value.termId;
    return {
      ok: true,
      value: {
        termId: result.value.termId,
        effective_size_policy: result.value.effective_size_policy,
        path: result.value.path,
      },
    };
  }

  input(data: string) {
    if (!this.termId) return;
    void invoke('term_input', { termId: this.termId, data }).catch(() => {});
  }

  /** xterm's onBinary: a latin1 string where each char is a byte. */
  inputBinary(data: string) {
    if (!this.termId) return;
    const bytes = new Uint8Array(data.length);
    for (let i = 0; i < data.length; i += 1) {
      bytes[i] = data.charCodeAt(i) & 0xff;
    }
    void invoke('term_input', { termId: this.termId, data: Array.from(bytes).map((b) => String.fromCharCode(b)).join('') }).catch(() => {});
  }

  resize(cols: number, rows: number) {
    if (!this.termId) return;
    void invoke('term_resize', { termId: this.termId, cols, rows }).catch(() => {});
  }

  async close() {
    if (!this.termId) return;
    const id = this.termId;
    this.termId = undefined;
    try {
      await invoke('term_close', { termId: id });
    } catch {
      // the reader may already have removed the session
    }
  }
}
