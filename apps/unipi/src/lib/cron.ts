/**
 * Scheduled runs on job files (app spec WP-A23, G-17): the UI's one seam
 * onto the `cron_*` Tauri commands (`src-tauri/src/cron.rs` → `unipi-host`'s
 * `cron_*` requests, `apps/unipi/crates/unipi-host/src/cron_serve.rs`), which the H8
 * merge answers from the job files under `<agent dir>/cron` and the embedded
 * scheduler `unipi-host serve` runs. Jobs are files written by the cron
 * extension and this seam alike; the store is the truth.
 *
 * Who may use these: this PC's own app always; a phone needs the `cron`
 * sharing feature in its per-device allow list first (UNI-248 pattern,
 * `lib/sharing.ts`), for every op below including the read-only ones.
 */
import { z } from 'zod';
import { hostCall } from './host/factory';
import { humanProviderError } from './model/humanError';
import {
  CronAddRequestSchema,
  CronEditRequestSchema,
  CronJobSchema,
  CronJobsDataSchema,
  CronLogDataSchema,
  CronParseDataSchema,
  CronRunStartedDataSchema,
  CronRunsDataSchema,
  CronSettingsDataSchema,
  CronSettingsSetRequestSchema,
  CronShowDataSchema,
  CronStatusDataSchema,
  CronStoppedDataSchema,
  type CronAddRequest,
  type CronEditRequest,
  type CronJob,
  type CronJobsData,
  type CronLogData,
  type CronNotify,
  type CronOpenSessionData,
  type CronParseData,
  type CronRun,
  type CronRunStartedData,
  type CronRunsData,
  type CronSettingsData,
  type CronSettingsSetRequest,
  type CronShowData,
  type CronStatusData,
  type CronStoppedData,
} from '@unipi/protocol';

export type {
  CronAddRequest,
  CronEditRequest,
  CronJob,
  CronJobsData,
  CronLogData,
  CronNotify,
  CronOpenSessionData,
  CronParseData,
  CronRun,
  CronRunStartedData,
  CronRunsData,
  CronSettingsData,
  CronSettingsSetRequest,
  CronShowData,
  CronStatusData,
  CronStoppedData,
};

export async function cronList(hostId: string) {
  return hostCall(hostId, 'cron_list', {}, CronJobsDataSchema);
}

export async function cronShow(hostId: string, id: string) {
  return hostCall(hostId, 'cron_show', { id }, CronShowDataSchema);
}

export async function cronRuns(hostId: string, id?: string, limit?: number) {
  return hostCall(hostId, 'cron_runs', { id, limit }, CronRunsDataSchema);
}

export async function cronLog(hostId: string, runId: string) {
  return hostCall(hostId, 'cron_log', { runId }, CronLogDataSchema);
}

export async function cronAdd(hostId: string, job: CronAddRequest) {
  return hostCall(hostId, 'cron_add', { job: CronAddRequestSchema.parse(job) }, CronJobSchema);
}

export async function cronEdit(hostId: string, id: string, patch: CronEditRequest) {
  return hostCall(hostId, 'cron_edit', { id, patch: CronEditRequestSchema.parse(patch) }, CronJobSchema);
}

export async function cronRemove(hostId: string, id: string) {
  return hostCall(hostId, 'cron_rm', { id }, z.unknown());
}

export async function cronEnable(hostId: string, id: string) {
  return hostCall(hostId, 'cron_enable', { id }, z.unknown());
}

export async function cronDisable(hostId: string, id: string) {
  return hostCall(hostId, 'cron_disable', { id }, z.unknown());
}

export async function cronRun(hostId: string, id: string) {
  return hostCall(hostId, 'cron_run', { id }, CronRunStartedDataSchema);
}

export async function cronStop(hostId: string, id: string) {
  return hostCall(hostId, 'cron_stop', { id }, CronStoppedDataSchema);
}

export async function cronPauseAll(hostId: string) {
  return hostCall(hostId, 'cron_pause_all', {}, z.unknown());
}

export async function cronResumeAll(hostId: string) {
  return hostCall(hostId, 'cron_resume_all', {}, z.unknown());
}

export async function cronStatus(hostId: string) {
  return hostCall(hostId, 'cron_status', {}, CronStatusDataSchema);
}

/** The add/edit form's live preview: resolves `schedule`/`at` (plain
 * language or machine form) to its canonical expression + the next 3
 * fires in `tz`. */
export async function cronParse(hostId: string, opts: { schedule?: string; at?: string; tz?: string }) {
  return hostCall(hostId, 'cron_parse', opts, CronParseDataSchema);
}

export async function cronSettingsGet(hostId: string) {
  return hostCall(hostId, 'cron_settings_get', {}, CronSettingsDataSchema);
}

export async function cronSettingsSet(hostId: string, patch: CronSettingsSetRequest) {
  return hostCall(hostId, 'cron_settings_set', { patch: CronSettingsSetRequestSchema.parse(patch) }, CronSettingsDataSchema);
}

/** Open a run's saved session as a live chat (WP-A23): the run's Oino session
 * carries the run id as its session id (the fired `oino -p --session-id <run
 * id>`), so once the run record has a `session_path` the host answers that id
 * and the app opens it in the session hub (host.md §9.8). */
const CronOpenedSchema = z.object({ session: z.string().min(1) });
export async function cronOpenSession(hostId: string, runId: string) {
  return hostCall(hostId, 'cron_open_session', { runId }, CronOpenedSchema);
}

/** Where a `/cron …` composer command goes (G-17): the reserved Cron page
 * instead of a text dump — `/cron` and `/cron list` open the page,
 * `/cron add` opens the page with the add-job form, anything else the page. */
export function cronCommandOf(text: string): string | null {
  const m = /^\/cron(?:\s+(\S+))?\s*$/.exec(text.trim());
  if (!m) return null;
  const arg = m[1]?.toLowerCase();
  return arg === 'add' || arg === 'new' ? '/cron?form=new' : '/cron';
}

/** UNI-319: what the form shows for a job's notify outcomes. */
export type NotifyMode = 'default' | 'choose' | 'never';
export const NOTIFY_EVENTS = ['done', 'failed', 'skipped'] as const;
export const NOTIFY_EVENT_LABEL: Record<string, string> = { done: 'Done', failed: 'Failed', skipped: 'Skipped' };

export function notifyModeOf(notify: CronNotify | undefined): NotifyMode {
  if (!notify?.events) return 'default';
  return notify.events.length === 0 ? 'never' : 'choose';
}

/** The form's choice → the wire's `notify` (`{}` = back to the PC's defaults).
 * Every channel picked = the default (absent), so a channel set up later is used too. */
export function notifyFromForm(mode: NotifyMode, events: readonly string[], channels: readonly string[], available: readonly string[]): CronNotify {
  const out: CronNotify = {};
  if (mode === 'never') out.events = [];
  else if (mode === 'choose') out.events = NOTIFY_EVENTS.filter((e) => events.includes(e));
  if (mode !== 'never' && available.length > 0 && channels.length > 0 && available.some((c) => !channels.includes(c))) {
    out.channels = available.filter((c) => channels.includes(c));
  }
  return out;
}

/** Plain words for a job's notify setting (detail sheet). */
export function notifyText(notify: CronNotify | undefined, pcDefault: readonly string[] = []): string {
  const mode = notifyModeOf(notify);
  if (mode === 'never') return 'never';
  const events = mode === 'choose' ? notify!.events! : pcDefault.length > 0 ? pcDefault : ['failed'];
  const words = events.map((e) => (NOTIFY_EVENT_LABEL[e] ?? e).toLowerCase()).join(', ');
  const via = notify?.channels?.length ? ` via ${notify.channels.join(', ')}` : '';
  return `${mode === 'default' ? 'PC default: ' : ''}${words}${via}`;
}

export type CronTranscriptEntry = {
  kind: 'prompt' | 'tool' | 'assistant' | 'error' | 'log';
  label?: string;
  text: string;
};

const providerOfModelKey = (model: string | undefined): string | undefined => {
  if (!model) return undefined;
  const slash = model.indexOf('/');
  return slash > 0 ? model.slice(0, slash) : undefined;
};

function messageBlocksText(content: unknown): string {
  if (typeof content === 'string') return content.trim();
  if (!Array.isArray(content)) return '';
  return content
    .map((p) => (p && typeof p === 'object' && (p as { type?: unknown }).type === 'text' && typeof (p as { text?: unknown }).text === 'string' ? (p as { text: string }).text : ''))
    .filter(Boolean)
    .join('\n')
    .trim();
}

function looksTruncatedJsonFragment(line: string): boolean {
  const t = line.trim();
  if (!t || t.startsWith('{')) return false;
  return (t.includes('","') || t.includes('":{') || t.includes('"parameters":{')) && (t.endsWith('}') || t.includes('\\n'));
}

/** G-07: humanizes `run.reason` ("oino exited with status exit status: 1" → cause or clean exit code). */
export function humanCronRunReason(reason: string | undefined, cause?: string, jobModel?: string): string | undefined {
  if (cause) return cause;
  const r = (reason ?? '').trim();
  if (!r) return undefined;
  const exit = /(?:oino\s+)?exited with status(?:\s+exit status:)?\s*(\d+)/i.exec(r);
  if (exit) return `Exited with code ${exit[1]}`;
  if (/^\d{3}\b|\{.*"message"/i.test(r)) return humanProviderError(r, providerOfModelKey(jobModel));
  return r;
}

/**
 * G-07: parses a cron run's raw `.log` (which may be `oino --mode json` JSONL
 * events + stderr, or plain text) into a clean cause line and human transcript,
 * never exposing raw JSON event blobs, system prompts, or tool schemas.
 */
export function parseCronRunLog(
  rawLog: string | undefined,
  runReason?: string,
  jobModel?: string,
): { cause?: string; entries: CronTranscriptEntry[]; transcript: string } {
  const raw = rawLog ?? '';
  if (!raw.trim()) {
    const cause = humanCronRunReason(runReason, undefined, jobModel);
    return { ...(cause ? { cause } : {}), entries: [], transcript: '' };
  }
  const lines = raw.split(/\r?\n/);
  const entries: CronTranscriptEntry[] = [];
  let sawJsonEvent = false;
  let errorProvider = providerOfModelKey(jobModel);
  let rawError: string | undefined;

  for (let idx = 0; idx < lines.length; idx++) {
    const line = lines[idx]!;
    const trimmed = line.trim();
    if (!trimmed) continue;
    if (idx === 0 && looksTruncatedJsonFragment(trimmed)) continue;

    if (trimmed.startsWith('{')) {
      try {
        const ev = JSON.parse(trimmed) as Record<string, unknown>;
        if (typeof ev.type === 'string') {
          sawJsonEvent = true;
          if (ev.type === 'message_end' || ev.type === 'turn_end') {
            const msg = (ev.message && typeof ev.message === 'object' ? ev.message : undefined) as Record<string, unknown> | undefined;
            if (!msg) continue;
            if (typeof msg.provider === 'string' && msg.provider) errorProvider = msg.provider;
            if (msg.stopReason === 'error' && typeof msg.errorMessage === 'string' && msg.errorMessage.trim()) {
              rawError = msg.errorMessage.trim();
            }
            if (ev.type !== 'message_end') continue;
            const role = msg.role;
            if (role === 'user') {
              const text = messageBlocksText(msg.content);
              if (text) entries.push({ kind: 'prompt', label: 'Prompt', text });
            } else if (role === 'assistant') {
              const content = Array.isArray(msg.content) ? msg.content : [];
              for (const part of content) {
                if (!part || typeof part !== 'object') continue;
                const p = part as Record<string, unknown>;
                if (p.type === 'toolCall' && typeof p.name === 'string') {
                  const args = p.arguments && typeof p.arguments === 'object' ? (p.arguments as Record<string, unknown>) : {};
                  const argPreview = typeof args.command === 'string' ? args.command : typeof args.path === 'string' ? args.path : '';
                  entries.push({ kind: 'tool', label: p.name, text: argPreview ? `${p.name}: ${argPreview}` : p.name });
                }
              }
              const text = messageBlocksText(msg.content);
              if (text) entries.push({ kind: 'assistant', label: 'Reply', text });
            } else if (role === 'toolResult') {
              const toolName = typeof msg.toolName === 'string' ? msg.toolName : 'tool';
              const isErr = msg.isError === true;
              const firstLine = messageBlocksText(msg.content).split('\n')[0]?.slice(0, 160) ?? '';
              entries.push({
                kind: isErr ? 'error' : 'tool',
                label: toolName,
                text: `${isErr ? '✗' : '✓'} ${toolName}${firstLine ? ` — ${firstLine}` : ''}`,
              });
            }
          }
          continue;
        }
      } catch {
        // Truncated trailing JSON event line (`agent_end` with huge system prompt cut at cap): drop it.
        if (/^\{"type":"(?:agent_|turn_|message_|session|tool_)/.test(trimmed)) {
          sawJsonEvent = true;
          continue;
        }
      }
    }

    if (/^(?:HTTP\s+)?\d{3}:\s*\{/i.test(trimmed) || /^\{\s*"(?:error|message)"/.test(trimmed)) {
      rawError ??= trimmed;
      continue;
    }
    entries.push({ kind: 'log', text: line });
  }

  const causeFromLog = rawError ? humanProviderError(rawError, errorProvider) : undefined;
  if (causeFromLog && !entries.some((e) => e.kind === 'error')) {
    entries.push({ kind: 'error', label: 'Error', text: causeFromLog });
  }
  const cause = humanCronRunReason(runReason, causeFromLog, jobModel);
  const transcript = sawJsonEvent
    ? entries.map((e) => (e.label && e.kind !== 'tool' ? `${e.label}: ${e.text}` : e.text)).join('\n\n')
    : entries.map((e) => e.text).join('\n');
  return { ...(cause ? { cause } : {}), entries, transcript };
}
