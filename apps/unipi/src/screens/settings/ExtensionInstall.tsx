/**
 * Settings › Extensions › "Install extension…" (ext-install): pick a package, hand its
 * path to the host (`host_cmd install_extension`), confirm a same-id replace, then —
 * the host cannot load a package into running services — "Restart agent host" and open
 * the new extension's own settings page once it is live.
 *
 * Where the package comes from:
 * - the desktop app on its own PC: a native folder picker (`extension.json` +
 *   `component.wasm`), or a `.zip` of that folder;
 * - the phone / a remote PC: a `.zip`, uploaded to the host through the blob channel
 *   (`uploadPicked` → a path under the host's blobs dir); a folder cannot be uploaded.
 */
import { For, Show, createSignal, type JSX } from 'solid-js';
import type { AppError } from '@unipi/protocol';
import { CheckCircle2, FolderOpen, Download, RotateCw, X, AlertTriangle } from 'lucide-solid';
import { installExtension, removeExtension, listExtensions, type InstallReply } from '../../settings/hostCmd';
import { isOwnPcFile, pickFolderNative } from '../../lib/openLocal';
import { uploadPicked } from '../../lib/blob';
import { canRestartHost, restartAgentHost } from '../../lib/host/restart';
import { ErrorBanner } from '../../components/ErrorBanner';

/** What the install card shows. */
export type InstallPhase =
  | { kind: 'idle' }
  | { kind: 'choose' }
  | { kind: 'busy'; text: string }
  | { kind: 'confirm'; path: string; reply: InstallReply }
  | { kind: 'restart'; text: string; id?: string; warnings: string[] }
  | { kind: 'waiting'; id?: string }
  | { kind: 'done'; text: string; warnings: string[] };

function asError(error: unknown, fallback: string): AppError {
  const e = error as Partial<AppError> & { message?: string };
  if (e && typeof e === 'object' && typeof e.code === 'string' && typeof e.message === 'string' && e.message) {
    return { code: e.code, message: e.message, hint: e.hint, retryable: !!e.retryable } as AppError;
  }
  return { code: 'E_INTERNAL', message: (e?.message as string) || fallback, retryable: false } as AppError;
}

/** The success line of an install reply. */
export function installedText(reply: InstallReply): string {
  const what = `${reply.name} v${reply.version}`;
  return reply.replaced !== undefined
    ? `Replaced ${reply.name}${reply.replaced ? ` v${reply.replaced}` : ''} with v${reply.version}.`
    : `Installed ${what}.`;
}

/** The confirm question for a same-id package. */
export function replaceQuestion(reply: InstallReply): string {
  const c = reply.confirm!;
  const old = c.installedVersion ? ` v${c.installedVersion}` : '';
  return `${c.name}${old} is already installed. Replace it with v${c.version}?`;
}

/** Polls the host until `id` is loaded (after a restart): `true` when it is. */
export async function waitForLoaded(
  hostId: string,
  id: string,
  opts: { tries?: number; delayMs?: number } = {},
): Promise<boolean> {
  const tries = opts.tries ?? 30;
  const delay = opts.delayMs ?? 1000;
  for (let i = 0; i < tries; i++) {
    const result = await listExtensions(hostId).catch(() => undefined);
    if (result?.ok && result.value.some((row) => row.id === id && row.loaded)) return true;
    if (i + 1 < tries) await new Promise((resolve) => setTimeout(resolve, delay));
  }
  return false;
}

const button = 'rounded-pill bg-surface-2 text-text-muted px-3 py-1.5 text-xs inline-flex items-center gap-1.5 disabled:opacity-50';
const primary = 'rounded-pill bg-shell text-face font-medium px-3 py-1.5 text-xs inline-flex items-center gap-1.5 disabled:opacity-50';

export function ExtensionInstallCard(props: {
  hostId: string;
  /** The list changed (installed/replaced/removed): refetch. */
  onChanged: () => void | Promise<unknown>;
  /** `id` is live: refetch and open its settings page; `false` when it has none. */
  onLive: (id: string) => Promise<boolean>;
  /** Poll options (tests). */
  wait?: { tries?: number; delayMs?: number };
  /** A remove from the list hands its outcome here (one restart affordance). */
  phase?: () => InstallPhase | undefined;
  setPhase?: (phase: InstallPhase) => void;
}) {
  const [own, setOwn] = createSignal<InstallPhase>({ kind: 'idle' });
  const phase = () => props.phase?.() ?? own();
  const setPhase = (next: InstallPhase) => (props.setPhase ? props.setPhase(next) : setOwn(next));
  const [error, setError] = createSignal<AppError | undefined>();
  let fileInput: HTMLInputElement | undefined;
  const desktop = () => isOwnPcFile(props.hostId);

  const install = async (path: string, replace = false) => {
    setError(undefined);
    setPhase({ kind: 'busy', text: 'Checking the package…' });
    const result = await installExtension(path, replace, props.hostId);
    if (!result.ok) {
      setError(result.error);
      setPhase({ kind: 'idle' });
      return;
    }
    const reply = result.value;
    if (!reply.installed && reply.confirm) {
      setPhase({ kind: 'confirm', path, reply });
      return;
    }
    await props.onChanged();
    if (reply.needsRestart) {
      setPhase({
        kind: 'restart',
        id: reply.id,
        warnings: reply.warnings,
        text: `${installedText(reply)} Restart the agent host to load it — its settings open next.`,
      });
      return;
    }
    await live(reply.id, installedText(reply), reply.warnings);
  };

  const live = async (id: string | undefined, text: string, warnings: string[]) => {
    if (id && (await props.onLive(id))) {
      setPhase({ kind: 'idle' });
      return;
    }
    setPhase({ kind: 'done', text, warnings });
  };

  const pickFolder = async () => {
    setError(undefined);
    let path: string | null;
    try {
      path = await pickFolderNative();
    } catch (e) {
      setError(asError(e, 'Could not open the folder picker'));
      return;
    }
    if (path) await install(path);
    else setPhase({ kind: 'idle' });
  };

  const pickZip = () => {
    setError(undefined);
    fileInput?.click();
  };

  const onFile: JSX.EventHandler<HTMLInputElement, Event> = async (event) => {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = '';
    if (!file) return;
    setPhase({ kind: 'busy', text: `Uploading ${file.name}…` });
    let path: string;
    try {
      path = (await uploadPicked(props.hostId, file)).path;
    } catch (e) {
      setError(asError(e, 'Could not upload the .zip'));
      setPhase({ kind: 'idle' });
      return;
    }
    await install(path);
  };

  const start = () => {
    setError(undefined);
    // The phone / a remote PC: only a .zip can travel.
    if (desktop()) setPhase({ kind: 'choose' });
    else pickZip();
  };

  const restart = async () => {
    const current = phase();
    const id = current.kind === 'restart' ? current.id : undefined;
    const warnings = current.kind === 'restart' ? current.warnings : [];
    setPhase({ kind: 'waiting', id });
    const ok = await restartAgentHost();
    if (!ok) {
      setError({ code: 'E_INTERNAL', message: 'Could not restart the agent host', hint: 'Quit and reopen Unipi to load the change.', retryable: false } as AppError);
      setPhase(current);
      return;
    }
    if (!id) {
      await props.onChanged();
      setPhase({ kind: 'done', text: 'Agent host restarted.', warnings });
      return;
    }
    const loaded = await waitForLoaded(props.hostId, id, props.wait);
    await props.onChanged();
    if (!loaded) {
      setPhase({ kind: 'done', text: `The agent host restarted but ${id} did not load — check the notices above.`, warnings });
      return;
    }
    await live(id, `${id} is loaded. It has no settings page.`, warnings);
  };

  return (
    <div class="px-4 py-3 border-t border-line" data-testid="ext-install">
      <input
        ref={fileInput}
        type="file"
        accept=".zip,application/zip,application/x-zip-compressed"
        class="hidden"
        data-testid="ext-install-file"
        onChange={onFile}
      />
      <Show when={error()} keyed>
        {(err) => (
          <div class="mb-3">
            <ErrorBanner error={err} dismiss={() => setError(undefined)} />
          </div>
        )}
      </Show>
      <Show when={phase().kind === 'idle'}>
        <div class="flex items-center gap-3 flex-wrap">
          <button type="button" class={button} data-testid="ext-install-open" onClick={start}>
            <Download size={13} /> Install extension…
          </button>
          <span class="text-xs text-text-dim">
            {desktop()
              ? 'A folder with extension.json + component.wasm, or a .zip of it'
              : 'A .zip of the extension folder — it uploads to the PC and installs there'}
          </span>
        </div>
      </Show>
      <Show when={phase().kind === 'choose'}>
        <div class="flex items-center gap-2 flex-wrap" data-testid="ext-install-choose">
          <button type="button" class={button} data-testid="ext-install-folder" onClick={() => void pickFolder()}>
            <FolderOpen size={13} /> Choose folder…
          </button>
          <button type="button" class={button} data-testid="ext-install-zip" onClick={pickZip}>
            <Download size={13} /> Choose .zip…
          </button>
          <button type="button" class={button} onClick={() => setPhase({ kind: 'idle' })} aria-label="Cancel">
            <X size={13} /> Cancel
          </button>
        </div>
      </Show>
      <Show when={phase().kind === 'busy' || phase().kind === 'waiting'}>
        <p class="text-xs text-text-muted" role="status" data-testid="ext-install-busy">
          {(() => {
            const p = phase();
            return p.kind === 'busy' ? p.text : 'Restarting the agent host…';
          })()}
        </p>
      </Show>
      <Show when={(() => { const p = phase(); return p.kind === 'confirm' ? p : undefined; })()} keyed>
        {(p) => (
          <div class="flex flex-col gap-2" role="alertdialog" data-testid="ext-install-confirm">
            <p class="text-[13px] text-text">{replaceQuestion(p.reply)}</p>
            <div class="flex gap-2">
              <button type="button" class={primary} data-testid="ext-install-replace" onClick={() => void install(p.path, true)}>
                Replace
              </button>
              <button type="button" class={button} onClick={() => setPhase({ kind: 'idle' })}>
                Cancel
              </button>
            </div>
          </div>
        )}
      </Show>
      <Show when={(() => { const p = phase(); return p.kind === 'restart' || p.kind === 'done' ? p : undefined; })()} keyed>
        {(p) => (
          <div class="flex flex-col gap-2" role="status" data-testid={p.kind === 'restart' ? 'ext-install-restart' : 'ext-install-done'}>
            <p class="text-[13px] text-text flex items-start gap-2">
              <CheckCircle2 size={15} class="shrink-0 mt-0.5 text-wm-green" />
              <span>{p.text}</span>
            </p>
            <For each={p.warnings}>
              {(warning) => (
                <p class="text-xs text-text-muted flex items-start gap-2" data-testid="ext-install-warning">
                  <AlertTriangle size={13} class="shrink-0 mt-0.5 text-amber" />
                  <span>{warning}</span>
                </p>
              )}
            </For>
            <div class="flex gap-2">
              <Show
                when={p.kind === 'restart'}
                fallback={
                  <button type="button" class={button} onClick={() => setPhase({ kind: 'idle' })}>
                    OK
                  </button>
                }
              >
                <Show
                  when={canRestartHost()}
                  fallback={<span class="text-xs text-text-dim" data-testid="ext-install-restart-manual">Restart Unipi on the PC to load it.</span>}
                >
                  <button type="button" class={primary} data-testid="ext-install-restart-host" onClick={() => void restart()}>
                    <RotateCw size={13} /> Restart agent host
                  </button>
                </Show>
                <button type="button" class={button} onClick={() => setPhase({ kind: 'idle' })}>
                  Later
                </button>
              </Show>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}

/** Remove a user-installed package; resolves to the restart phase or an error. */
export async function removeInstalled(hostId: string, id: string, name: string): Promise<{ ok: true; phase: InstallPhase } | { ok: false; error: AppError }> {
  const result = await removeExtension(id, hostId);
  if (!result.ok) return { ok: false, error: result.error };
  const tail = result.value.bundledRemains ? ' The version that ships with the app takes over again.' : '';
  return {
    ok: true,
    phase: result.value.needsRestart
      ? { kind: 'restart', warnings: [], text: `Removed ${name}. Restart the agent host to unload it.${tail}` }
      : { kind: 'done', warnings: [], text: `Removed ${name}.${tail}` },
  };
}
