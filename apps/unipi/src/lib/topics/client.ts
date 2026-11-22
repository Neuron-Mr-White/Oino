/** Shared per-host topic connection. The session hub owns/reconnects the events stream.
 * Unsupported topics retain reviewed polling; no capability means old-host fallback.
 */
import type { EventsStream, HubEvent } from '../host/client';

type Cursor = { epoch: number; seq: number };
type Reader = { topic: string; visible: () => boolean; refresh: () => void; fallback: () => () => void; stop?: () => void };

export class TopicClient {
  private stream?: EventsStream;
  private capabilities = new Set<string>();
  private failed = new Set<string>();
  private readers = new Set<Reader>();
  private active = new Set<string>();
  private cursors = new Map<string, Cursor>();
  constructor(private hidden = () => document.hidden) {}

  bind(stream?: EventsStream) {
    this.stream = stream;
    this.capabilities.clear();
    this.failed.clear();
    this.active.clear();
    this.sync();
  }

  event(event: HubEvent) {
    if (event.kind === 'hello') {
      this.capabilities = new Set(event.capabilities);
      this.sync();
    } else if (event.kind === 'topic' && this.active.has(event.topic)) {
      if ((event.snapshot as { status?: string } | undefined)?.status === 'error') {
        this.failed.add(event.topic);
        this.cursors.delete(event.topic);
        this.sync();
        return;
      }
      const previous = this.cursors.get(event.topic);
      if (event.snapshot === undefined && previous &&
          (event.epoch !== previous.epoch || event.seq !== previous.seq + 1)) {
        if (event.epoch === previous.epoch && event.seq <= previous.seq) return;
        this.cursors.delete(event.topic);
        this.stream?.topics?.([{ topic: event.topic }], []);
        return;
      }
      this.cursors.set(event.topic, { epoch: event.epoch, seq: event.seq });
      for (const reader of this.readers) if (reader.topic === event.topic && this.enabled(reader)) reader.refresh();
    }
  }

  private enabled(reader: Reader) {
    return reader.visible() && (!this.hidden() || reader.topic === 'needs_you' || reader.topic === 'notifications');
  }

  sync() {
    const wanted = new Set<string>();
    for (const reader of this.readers) {
      const supported = !this.failed.has(reader.topic) && !!this.stream?.topics && this.capabilities.has('topics/1') &&
        this.capabilities.has(`topic:${reader.topic.split(':')[0]}`);
      if (this.enabled(reader) && supported) wanted.add(reader.topic);
      if (!this.enabled(reader) || supported) { reader.stop?.(); reader.stop = undefined; }
      else if (!reader.stop) reader.stop = reader.fallback();
    }
    const add = [...wanted].filter((topic) => !this.active.has(topic));
    const remove = [...this.active].filter((topic) => !wanted.has(topic));
    if (add.length || remove.length) this.stream?.topics?.(add.map((topic) => ({ topic, since: this.cursors.get(topic) })), remove);
    this.active = wanted;
  }

  watch(topic: string, refresh: () => void, fallback: () => () => void, visible = () => true) {
    const reader: Reader = { topic, refresh, fallback, visible };
    this.readers.add(reader);
    this.sync();
    return () => { reader.stop?.(); this.readers.delete(reader); this.sync(); };
  }
}

const clients = new Map<string, TopicClient>();
export function topicsFor(host: string) {
  let client = clients.get(host);
  if (!client) { client = new TopicClient(); clients.set(host, client); }
  return client;
}
if (typeof document !== 'undefined') document.addEventListener('visibilitychange', () => {
  for (const client of clients.values()) client.sync();
});
