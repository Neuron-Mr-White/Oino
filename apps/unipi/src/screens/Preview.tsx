/**
 * Preview (UNI-213, forwarding phase 1): open a web app running on the PC
 * (a Vite dev server, `python -m http.server`, …) on this device.
 *
 * Picker: the PC's listening ports (`ports_list`, web servers first),
 * refresh, a manual port box, and the forwards already running. Picking a
 * port starts a loopback forward (`forward_start`) and opens the viewer: a
 * frame on `http://127.0.0.1:<localPort>/` with a URL bar, reload, open in
 * the external browser and stop. Every connection the page makes (HTTP,
 * WebSocket, Vite HMR) rides its own iroh stream to the PC.
 *
 * States: needs the app (plain web), no PCs, forwarding off on the PC
 * (with the exact command to enable it), port not allowed, no ports,
 * port closed, connection lost.
 *
 * Browser mode (UNI-215, `?mode=browser`): drive a dedicated desktop
 * browser on the PC over CDP — see `BrowserView.tsx`.
 *
 * Desktop streaming (UNI-216) is deferred (D-15, WP-A1 removed it).
 *
 * WP-A3 (Z-104): this screen is the phone's. On the desktop shell the ports
 * list is "Local ports" in the side pane `+` launcher and a port opens as a
 * `forward` tab (`ForwardViewer` embedded, components/panes/ForwardTab.tsx).
 */
import { createEffect, createSignal, For, onCleanup, Show, type JSX } from 'solid-js';
import { useNavigate, useSearchParams } from '@solidjs/router';
import {
  ArrowLeft,
  ChevronRight,
  Copy,
  ExternalLink,
  Globe,
  Home,
  Monitor,
  Plug,
  RefreshCw,
  RotateCw,
  Square,
  TriangleAlert,
  Unplug,
} from 'lucide-solid';
import type { AppError, FeatureKind, ListeningPort, PortsListData } from '@unipi/protocol';
import { useHosts } from '../hosts/context';
import type { Result } from '../lib/ipc';
import { pushBack } from '../lib/back';
import { copyText, openExternal } from '../lib/board/share';
import { ErrorBanner } from '../components/ErrorBanner';
import { isUnreachable, OfflineState } from '../components/EmptyState';
import { BoardSheet } from '../components/board/BoardSheet';
import { PageHeader } from '../components/PageHeader';
import { PcChip } from '../components/PcChip';
import { NoHostState } from '../components/EmptyState';
import { LeaveButton, LeaveSheet } from '../components/LeaveSheet';
import { deviceNoun, setAddHostOpen } from '../components/Pairing';
import { desktopShell } from '../lib/layout';
import {
  canForward,
  ENABLE_COMMAND,
  ENABLE_PORT_COMMAND,
  forwardList,
  forwardStart,
  forwardStop,
  parsePortInput,
  pickerState,
  portLabel,
  portsList,
  previewUrl,
  urlPath,
  viewerBanner,
  type ForwardInfo,
} from '../lib/forward';
import { BrowserPanel, BrowserViewer } from './BrowserView';
import { UnavailableNotice } from '../components/Requires';
import { FeatureOff } from '../components/FeatureOff';
import { Button, IconButton, ListGroup, ListRow, RowIcon, Sheet } from '../components/phone';
import '../components/Requires.css';
import './preview.css';

/** The preview iframe may do everything except navigate the app itself away (UNI-224). */
// No allow-popups either: the Android WebView has no multi-window support, so a
// `target=_blank` link or window.open() from the previewed page loaded over the
// app itself, the same dead end as a top navigation.
export const PREVIEW_SANDBOX = 'allow-scripts allow-same-origin allow-forms allow-modals allow-downloads allow-pointer-lock';

/** How often the viewer checks its forward for failed connections. */
const POLL_MS = 2000;

export default function Preview() {
  return (
    <div class="preview-screen" data-testid="preview-screen">
      <Show when={canForward()} fallback={<NeedsApp />}>
        <PreviewApp />
      </Show>
    </div>
  );
}

/** Preview's page header: the shared PageHeader (UNI-218 F1). Preview is a phone-only
 * screen now (Z-104): pushed over the session list, so it gets a back arrow. */
function PreviewHead(props: { onBack: () => void; chip?: JSX.Element; children?: JSX.Element }) {
  return (
    <PageHeader
      class="preview-head"
      title="Preview"
      lead={
        <Show when={!desktopShell() || props.chip} fallback={undefined}>
          <Show when={!desktopShell()}>
            <button type="button" class="ghost-icon-button preview-back" aria-label="Back" onClick={props.onBack}>
              <ArrowLeft size={20} />
            </button>
          </Show>
          {props.chip}
        </Show>
      }
      tools={props.children}
    />
  );
}

function NeedsApp() {
  // Web build (UNI-218 F59): the same page notice as Sessions / Chat / Files.
  const navigate = useNavigate();
  return (
    <div class="page-screen">
      <PreviewHead onBack={() => navigate('/')} />
      <div data-testid="preview-needs-app"><UnavailableNotice cap="portForward" /></div>
    </div>
  );
}

function Notice(props: { icon: JSX.Element; title: string; tone?: 'warn' | 'error'; testId?: string; children: JSX.Element }) {
  return (
    <section class="preview-notice" data-tone={props.tone} data-testid={props.testId} role="status">
      <div class="preview-notice-icon">{props.icon}</div>
      <h2>{props.title}</h2>
      <div class="preview-notice-text">{props.children}</div>
    </section>
  );
}

function CommandBox(props: { command: string }) {
  const [copied, setCopied] = createSignal(false);
  return (
    <div class="preview-command">
      <code>{props.command}</code>
      <button
        type="button"
        class="ghost-icon-button"
        aria-label="Copy command"
        onClick={async () => {
          setCopied(await copyText(props.command));
          setTimeout(() => setCopied(false), 1500);
        }}
      >
        <Show when={copied()} fallback={<Copy size={16} />}>
          <span class="preview-copied">Copied</span>
        </Show>
      </button>
    </div>
  );
}

function PreviewApp() {
  const hosts = useHosts();
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams<{ host?: string; mode?: string }>();
  const mode = () => (params.mode === 'browser' ? 'browser' : 'ports');
  const [browsing, setBrowsing] = createSignal<{ hostId: string; url?: string; target?: string } | undefined>();
  const [hostSheet, setHostSheet] = createSignal(false);
  const hostId = () => {
    const wanted = params.host;
    if (wanted && hosts.hosts().some((h) => h.id === wanted)) return wanted;
    return hosts.selectedId() ?? hosts.hosts()[0]?.id;
  };
  const hostName = (id: string | undefined) => hosts.hosts().find((h) => h.id === id)?.name ?? 'PC';
  /** UNI-233: this PC's own host (local socket): web apps are already on 127.0.0.1, its screen is right here. */
  const isLocal = (id: string | undefined) => hosts.hosts().find((h) => h.id === id)?.transport === 'local';

  const [ports, setPorts] = createSignal<Result<PortsListData> | undefined>();
  const [refreshing, setRefreshing] = createSignal(false);
  const [forwards, setForwards] = createSignal<ForwardInfo[]>([]);
  const [starting, setStarting] = createSignal<number | undefined>();
  const [startError, setStartError] = createSignal<{ port: number; error: AppError } | undefined>();
  const [manual, setManual] = createSignal('');
  const [manualError, setManualError] = createSignal<string | undefined>();
  const [viewing, setViewing] = createSignal<ForwardInfo | undefined>();
  // UNI-253: a phone can turn off a shared feature it has.
  const [leaving, setLeaving] = createSignal<FeatureKind | undefined>();

  async function refresh() {
    const id = hostId();
    if (!id) return;
    setRefreshing(true);
    const [list, running] = await Promise.all([portsList(id), forwardList()]);
    setRefreshing(false);
    if (id !== hostId()) return;
    setPorts(list);
    if (running.ok) setForwards(running.value);
  }
  createEffect(() => {
    hostId();
    if (mode() !== 'ports') return;
    setPorts(undefined);
    setStartError(undefined);
    void refresh();
  });

  async function open(port: number) {
    const id = hostId();
    if (!id || starting() !== undefined) return;
    if (isLocal(id)) {
      // Same machine: no tunnel — the page is on this PC's loopback already.
      setViewing({ id: `local-${port}`, hostId: id, remotePort: port, localPort: port, url: `http://127.0.0.1:${port}/`, startedAt: Date.now(), active: 0, total: 0 } as ForwardInfo);
      return;
    }
    setStarting(port);
    setStartError(undefined);
    const result = await forwardStart(id, port);
    setStarting(undefined);
    if (!result.ok) {
      setStartError({ port, error: result.error });
      // The PC may just have turned forwarding off: refresh the picker.
      if (result.error.code === 'E_FORWARD_DISABLED') setPorts({ ok: false, error: result.error });
      return;
    }
    setViewing(result.value);
    const running = await forwardList();
    if (running.ok) setForwards(running.value);
  }

  async function stop(id: string) {
    if (!id.startsWith('local-')) await forwardStop(id);
    if (viewing()?.id === id) setViewing(undefined);
    const running = await forwardList();
    if (running.ok) setForwards(running.value);
  }

  function submitManual(event: Event) {
    event.preventDefault();
    const parsed = parsePortInput(manual());
    if (!parsed.ok) {
      setManualError(parsed.message);
      return;
    }
    setManualError(undefined);
    void open(parsed.port);
  }

  const hostForwards = () => forwards().filter((f) => f.hostId === hostId());

  return (
    <Show
      when={hosts.hosts().length > 0}
      fallback={
        <>
          <PreviewHead onBack={() => navigate('/')} />
          <div class="preview-body">
            <div data-testid="preview-no-hosts">
              <NoHostState what="its web apps and browser" onAdd={() => setAddHostOpen(true)} />
            </div>
          </div>
        </>
      }
    >
      <Show when={browsing()}>
        {(b) => (
          <BrowserViewer
            hostId={b().hostId}
            hostName={hostName(b().hostId)}
            url={b().url}
            target={b().target}
            CommandBox={CommandBox}
            onClose={() => setBrowsing(undefined)}
          />
        )}
      </Show>
      <Show
        when={viewing()}
        fallback={
          <>
            <PreviewHead
              onBack={() => navigate('/')}
              chip={<PcChip name={hostName(hostId())} state={ports() && !ports()!.ok && isUnreachable((ports() as { ok: false; error: AppError }).error.code) ? 'error' : 'ok'} label="Choose a PC" onClick={() => setHostSheet(true)} />}
            >
              <Show when={mode() === 'ports'}>
                <button type="button" class="ghost-icon-button" aria-label="Refresh ports" onClick={() => void refresh()}>
                  <RefreshCw size={19} class={refreshing() ? 'refresh-spinning' : ''} />
                </button>
              </Show>
            </PreviewHead>
            <div class="preview-body">
              <div class="seg preview-modes" role="tablist" aria-label="Preview mode">
                <button type="button" role="tab" class="seg-option" aria-selected={mode() === 'ports'} aria-checked={mode() === 'ports'} onClick={() => setParams({ mode: undefined }, { replace: true })}>
                  <Plug size={15} /> Web apps
                </button>
                <button type="button" role="tab" class="seg-option" aria-selected={mode() === 'browser'} aria-checked={mode() === 'browser'} onClick={() => setParams({ mode: 'browser' }, { replace: true })}>
                  <Monitor size={15} /> Browser
                </button>
              </div>
              <Show when={mode() === 'browser' && hostId()}>
                {(id) => (
                  <BrowserPanel
                    hostId={id()}
                    hostName={hostName(id())}
                    Notice={Notice}
                    CommandBox={CommandBox}
                    onOpen={(args) => setBrowsing({ hostId: id(), ...args })}
                  />
                )}
              </Show>
              <Show when={mode() === 'ports'}>

              <Show when={hostForwards().length > 0}>
                <h2 class="section-label">Open</h2>
                <div class="preview-list" data-testid="preview-running">
                  <For each={hostForwards()}>
                    {(f) => (
                      <div class="preview-row">
                        <button type="button" class="preview-row-main" onClick={() => setViewing(f)}>
                          <span class="preview-port">:{f.remotePort}</span>
                          <span class="preview-row-text">
                            <span class="preview-row-name">Open preview</span>
                            <span class="preview-row-meta">
                              this device :{f.localPort} · {f.active} live connection{f.active === 1 ? '' : 's'}
                            </span>
                          </span>
                        </button>
                        <button type="button" class="ghost-icon-button" aria-label={`Stop forwarding port ${f.remotePort}`} onClick={() => void stop(f.id)}>
                          <Square size={16} />
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </Show>

              <PortPicker
                state={pickerState(ports())}
                starting={starting()}
                startError={startError()}
                onOpen={(port) => void open(port)}
                onRetry={() => void refresh()}
                hostName={hostName(hostId())}
                hostId={hostId()}
              />

              <form class="preview-manual" onSubmit={submitManual}>
                <label class="section-label" for="preview-port-input">Other port</label>
                <div class="preview-manual-row">
                  <input
                    id="preview-port-input"
                    class="preview-input"
                    inputmode="numeric"
                    placeholder="e.g. 5173"
                    value={manual()}
                    aria-invalid={manualError() ? 'true' : undefined}
                    onInput={(e) => {
                      setManual(e.currentTarget.value);
                      setManualError(undefined);
                    }}
                  />
                  <button type="submit" class="preview-btn primary" disabled={starting() !== undefined}>
                    Open
                  </button>
                </div>
                <Show when={manualError()}>
                  <p class="preview-field-error" role="alert">{manualError()}</p>
                </Show>
              </form>
              </Show>
            </div>
            {/* UNI-253: stop using what this PC gives this phone. */}
            <div class="preview-leave">
              <LeaveButton onClick={() => setLeaving('forward')} label="Turn off web apps for this phone" testId="forward-leave" />
            </div>
            <LeaveSheet open={!!leaving()} onClose={() => setLeaving(undefined)} hostId={hostId() ?? ''} feature="forward" allowed />
            <BoardSheet open={hostSheet()} onClose={() => setHostSheet(false)} label="Choose a PC" testId="preview-host-sheet">
              <div class="settings-group rounded-card bg-surface-2/60">
                <For each={hosts.hosts()}>
                  {(host) => (
                    <button
                      type="button"
                      class="picker-row"
                      aria-current={hostId() === host.id ? 'true' : undefined}
                      onClick={() => {
                        hosts.selectHost(host.id);
                        setParams({ host: host.id }, { replace: true });
                        setHostSheet(false);
                      }}
                    >
                      <span class="settings-icon"><Home size={16} /></span>
                      <span class="min-w-0 flex-1 text-left truncate">{host.name}</span>
                    </button>
                  )}
                </For>
              </div>
            </BoardSheet>
          </>
        }
      >
        {(current) => (
          <Viewer
            forward={current()}
            hostName={hostName(current().hostId)}
            onClose={() => {
              setViewing(undefined);
              void refresh();
            }}
            onStop={() => void stop(current().id)}
          />
        )}
      </Show>
    </Show>
  );
}

/**
 * Settings › <PC> › Ports & previews (phone spec §3.12): the body of the
 * `/settings/ports` pushed page (C renders it inside its PageScaffold). Two
 * groups instead of the old segmented control — "Web apps on <PC>" (the PC's
 * listening ports; tap opens the preview, open forwards first with Stop) and
 * "Browser tabs" (the in-app browser on the PC). Follows the selected PC.
 * Off state = the shared FeatureOff card (same as Cron, PH-38).
 */
export function PortsSection() {
  const hosts = useHosts();
  const hostId = () => hosts.selectedId() ?? hosts.hosts()[0]?.id;
  const hostName = (id: string | undefined) => hosts.hosts().find((h) => h.id === id)?.name ?? 'PC';
  const isLocal = (id: string | undefined) => hosts.hosts().find((h) => h.id === id)?.transport === 'local';
  const [ports, setPorts] = createSignal<Result<PortsListData> | undefined>();
  const [refreshing, setRefreshing] = createSignal(false);
  const [forwards, setForwards] = createSignal<ForwardInfo[]>([]);
  const [starting, setStarting] = createSignal<number | undefined>();
  const [startError, setStartError] = createSignal<{ port: number; error: AppError } | undefined>();
  const [viewing, setViewing] = createSignal<ForwardInfo | undefined>();
  const [browsing, setBrowsing] = createSignal<{ hostId: string; url?: string; target?: string } | undefined>();
  const [otherOpen, setOtherOpen] = createSignal(false);
  const [manual, setManual] = createSignal('');
  const [manualError, setManualError] = createSignal<string | undefined>();
  const [leaving, setLeaving] = createSignal(false);

  async function refresh() {
    const id = hostId();
    if (!id) return;
    setRefreshing(true);
    const [list, running] = await Promise.all([portsList(id), forwardList()]);
    setRefreshing(false);
    if (id !== hostId()) return;
    setPorts(list);
    if (running.ok) setForwards(running.value);
  }
  createEffect(() => {
    hostId();
    setPorts(undefined);
    setStartError(undefined);
    void refresh();
  });
  async function open(port: number) {
    const id = hostId();
    if (!id || starting() !== undefined) return;
    if (isLocal(id)) {
      setViewing({ id: `local-${port}`, hostId: id, remotePort: port, localPort: port, url: `http://127.0.0.1:${port}/`, startedAt: Date.now(), active: 0, total: 0 } as ForwardInfo);
      return;
    }
    setStarting(port);
    setStartError(undefined);
    const result = await forwardStart(id, port);
    setStarting(undefined);
    if (!result.ok) {
      setStartError({ port, error: result.error });
      if (result.error.code === 'E_FORWARD_DISABLED') setPorts({ ok: false, error: result.error });
      return;
    }
    setViewing(result.value);
    const running = await forwardList();
    if (running.ok) setForwards(running.value);
  }
  async function stop(id: string) {
    if (!id.startsWith('local-')) await forwardStop(id);
    if (viewing()?.id === id) setViewing(undefined);
    const running = await forwardList();
    if (running.ok) setForwards(running.value);
  }
  const submitOther = () => {
    const parsed = parsePortInput(manual());
    if (!parsed.ok) return setManualError(parsed.message);
    setManualError(undefined);
    setOtherOpen(false);
    void open(parsed.port);
  };
  const hostForwards = () => forwards().filter((f) => f.hostId === hostId());
  const state = () => pickerState(ports());
  const portRow = (p: ListeningPort) => (
    <ListRow
      testId={`port-row-${p.port}`}
      leading={<RowIcon icon={p.http ? Globe : Plug} tint={p.allowed ? 'accent' : 'neutral'} />}
      title={`:${p.port} · ${portLabel(p)}`}
      subtitle={p.allowed ? (p.cmd ?? `bound to ${p.bind}`) : 'Not allowed on the PC'}
      trailing={starting() === p.port ? <RefreshCw size={18} class="refresh-spinning" /> : undefined}
      chevron={starting() !== p.port}
      disabled={starting() !== undefined}
      onClick={() => void open(p.port)}
    />
  );

  return (
    <div class="ports-section" data-testid="ports-section">
      <Show when={canForward()} fallback={<div data-testid="preview-needs-app"><UnavailableNotice cap="portForward" /></div>}>
        <Show when={hosts.hosts().length > 0} fallback={<NoHostState what="its web apps and browser" onAdd={() => setAddHostOpen(true)} />}>
          <Show when={hostForwards().length > 0}>
            <ListGroup title="Open now" inset>
              <For each={hostForwards()}>
                {(f) => (
                  <ListRow
                    leading={<RowIcon icon={Globe} tint="green" />}
                    title={`:${f.remotePort}`}
                    subtitle={`${f.active} live connection${f.active === 1 ? '' : 's'}`}
                    onClick={() => setViewing(f)}
                    trailing={<IconButton icon={Square} label={`Stop forwarding port ${f.remotePort}`} onClick={(e) => (e.stopPropagation(), void stop(f.id))} />}
                  />
                )}
              </For>
            </ListGroup>
          </Show>
          <Show when={startError() && startError()!.error.code !== 'E_FORWARD_DISABLED' && startError()}>
            {(failure) => (
              <div class="preview-inline-error ports-inline" role="alert" data-testid="preview-start-error">
                <TriangleAlert size={18} />
                <div>
                  <p>{failure().error.code === 'E_NOT_FOUND' ? `Port ${failure().port} is closed: nothing is listening there on the PC.` : failure().error.message}</p>
                  <Show when={failure().error.code === 'E_UNAUTHORIZED'}>
                    <p class="preview-muted">Allow it on the PC:</p>
                    <CommandBox command={ENABLE_PORT_COMMAND(failure().port)} />
                  </Show>
                </div>
              </div>
            )}
          </Show>
          <Show when={state().kind === 'disabled' && hostId()}>
            {(id) => (
              <div class="ports-off" data-testid="preview-disabled">
                <FeatureOff feature="forward" hostId={id()} hostName={hostName(id())} CommandBox={CommandBox} onChanged={() => void refresh()} />
              </div>
            )}
          </Show>
          <Show when={state().kind === 'error' && (state() as { error: AppError }).error}>
            {(err) => (
              <div class="ports-off">
                <Show when={isUnreachable(err().code)} fallback={<ErrorBanner error={err()} retry={() => void refresh()} />}>
                  <OfflineState compact pc={hostName(hostId())} message={err().message} hint={err().hint} onRetry={() => void refresh()} />
                </Show>
              </div>
            )}
          </Show>
          <Show when={state().kind === 'loading' || state().kind === 'empty' || state().kind === 'ports'}>
            <ListGroup
              title={`Web apps on ${hostName(hostId())}`}
              inset
              footer={
                <Show when={state().kind === 'ports' && !(state() as { any: boolean }).any}>
                  This PC only forwards {(state() as { allowed: number[] }).allowed.length ? `ports ${(state() as { allowed: number[] }).allowed.join(', ')}` : 'listed ports'}.
                </Show>
              }
            >
              <Show when={state().kind === 'loading'}>
                <ListRow leading={<RefreshCw size={20} class="refresh-spinning" />} title="Looking for ports on the PC…" />
              </Show>
              <Show when={state().kind === 'empty'}>
                <ListRow leading={<RowIcon icon={Plug} />} title="No open ports" subtitle="Start a dev server on the PC, like npm run dev" wrap />
              </Show>
              <Show when={state().kind === 'ports' && (state() as Extract<ReturnType<typeof pickerState>, { kind: 'ports' }>)}>
                {(st) => (
                  <>
                    <For each={st().web}>{portRow}</For>
                    <For each={st().other}>{portRow}</For>
                  </>
                )}
              </Show>
              <ListRow testId="ports-other" leading={<RowIcon icon={Plug} />} title="Open another port" chevron onClick={() => setOtherOpen(true)} />
              <ListRow testId="ports-refresh" leading={<RowIcon icon={RefreshCw} />} title={refreshing() ? 'Refreshing…' : 'Refresh'} onClick={() => void refresh()} />
            </ListGroup>
          </Show>
          <Show when={hostId()}>
            {(id) => (
              <ListGroup title="Browser tabs" inset>
                <div class="ports-browser">
                  <BrowserPanel hostId={id()} hostName={hostName(id())} Notice={Notice} CommandBox={CommandBox} onOpen={(args) => setBrowsing({ hostId: id(), ...args })} />
                </div>
              </ListGroup>
            )}
          </Show>
          <div class="preview-leave">
            <LeaveButton onClick={() => setLeaving(true)} label="Turn off web apps for this phone" testId="forward-leave" />
          </div>
          <LeaveSheet open={leaving()} onClose={() => setLeaving(false)} hostId={hostId() ?? ''} feature="forward" allowed />
          <Sheet
            open={otherOpen()}
            onClose={() => setOtherOpen(false)}
            label="Open another port"
            title="Open another port"
            footer={
              <Button kind="primary" block disabled={starting() !== undefined} onClick={submitOther} testId="ports-other-open">
                Open
              </Button>
            }
          >
            <label class="ph-field-label" for="ports-other-input">Port on {hostName(hostId())}</label>
            <input
              id="ports-other-input"
              class="preview-input ph-input"
              inputmode="numeric"
              enterkeyhint="go"
              placeholder="e.g. 5173"
              autofocus
              value={manual()}
              aria-invalid={manualError() ? 'true' : undefined}
              onInput={(e) => (setManual(e.currentTarget.value), setManualError(undefined))}
              onKeyDown={(e) => e.key === 'Enter' && submitOther()}
            />
            <Show when={manualError()}>
              <p class="preview-field-error" role="alert">{manualError()}</p>
            </Show>
          </Sheet>
          <Show when={browsing()}>
            {(b) => <BrowserViewer hostId={b().hostId} hostName={hostName(b().hostId)} url={b().url} target={b().target} CommandBox={CommandBox} onClose={() => setBrowsing(undefined)} />}
          </Show>
          <Show when={viewing()}>
            {(current) => (
              <Viewer
                forward={current()}
                hostName={hostName(current().hostId)}
                onClose={() => {
                  setViewing(undefined);
                  void refresh();
                }}
                onStop={() => void stop(current().id)}
              />
            )}
          </Show>
        </Show>
      </Show>
    </div>
  );
}

function PortPicker(props: {
  state: ReturnType<typeof pickerState>;
  starting: number | undefined;
  startError: { port: number; error: AppError } | undefined;
  onOpen: (port: number) => void;
  onRetry: () => void;
  hostName?: string;
  hostId?: string;
}) {
  const startProblem = () => {
    const failure = props.startError;
    if (!failure || failure.error.code === 'E_FORWARD_DISABLED') return undefined;
    return failure;
  };
  return (
    <>
      <Show when={startProblem()}>
        {(failure) => (
          <div class="preview-inline-error" role="alert" data-testid="preview-start-error">
            <TriangleAlert size={18} />
            <div>
              <p>
                {failure().error.code === 'E_NOT_FOUND'
                  ? `Port ${failure().port} is closed: nothing is listening there on the PC.`
                  : failure().error.message}
              </p>
              <Show when={failure().error.code === 'E_UNAUTHORIZED'}>
                <p class="preview-muted">Allow it on the PC:</p>
                <CommandBox command={ENABLE_PORT_COMMAND(failure().port)} />
              </Show>
            </div>
          </div>
        )}
      </Show>
      <Show when={props.state.kind === 'loading'}>
        <div class="preview-loading" data-testid="preview-loading">
          <RefreshCw size={18} class="refresh-spinning" /> Looking for ports on the PC…
        </div>
      </Show>
      <Show when={props.state.kind === 'disabled' && props.hostId}>
        {(id) => (
          <div data-testid="preview-disabled">
            <FeatureOff feature="forward" hostId={id()} hostName={props.hostName ?? 'the PC'} CommandBox={CommandBox} onChanged={props.onRetry} />
          </div>
        )}
      </Show>
      <Show when={props.state.kind === 'error' && props.state}>
        {(state) => (
          <Show when={isUnreachable(state().error.code)} fallback={<ErrorBanner error={state().error} retry={props.onRetry} />}>
            <OfflineState compact pc={props.hostName} message={state().error.message} hint={state().error.hint} onRetry={props.onRetry} />
          </Show>
        )}
      </Show>
      <Show when={props.state.kind === 'empty' && props.state}>
        {(state) => (
          <Notice icon={<Plug size={22} />} title="No open ports on the PC" testId="preview-empty">
            <p>Nothing is listening on the PC right now. Start your dev server (for example <code>npm run dev</code>) and refresh.</p>
            <Show when={!state().any && state().allowed.length > 0}>
              <p class="preview-muted">Allowed ports: {state().allowed.join(', ')}</p>
            </Show>
            <button type="button" class="preview-btn" onClick={props.onRetry}>
              <RefreshCw size={16} /> Refresh
            </button>
          </Notice>
        )}
      </Show>
      <Show when={props.state.kind === 'ports' && props.state}>
        {(state) => (
          <>
            <Show when={state().web.length > 0}>
              <h2 class="section-label">Web servers</h2>
              <PortList ports={state().web} starting={props.starting} onOpen={props.onOpen} />
            </Show>
            <Show when={state().other.length > 0}>
              <h2 class="section-label">Other ports</h2>
              <PortList ports={state().other} starting={props.starting} onOpen={props.onOpen} />
            </Show>
            <Show when={!state().any}>
              <p class="preview-muted preview-allow-note">
                This PC only forwards {state().allowed.length ? `ports ${state().allowed.join(', ')}` : 'listed ports'}. Others
                need <code>unipi-host forward enable --port N</code> on the PC.
              </p>
            </Show>
          </>
        )}
      </Show>
    </>
  );
}

function PortList(props: { ports: ListeningPort[]; starting: number | undefined; onOpen: (port: number) => void }) {
  return (
    <div class="preview-list">
      <For each={props.ports}>
        {(p) => (
          <button
            type="button"
            class="preview-row preview-row-main"
            data-allowed={p.allowed ? 'true' : 'false'}
            aria-label={`Open port ${p.port}`}
            disabled={props.starting !== undefined}
            onClick={() => props.onOpen(p.port)}
          >
            <span class="preview-port">:{p.port}</span>
            <span class="preview-row-text">
              <span class="preview-row-name">
                {portLabel(p)}
                <Show when={!p.allowed}>
                  <span class="preview-badge">not allowed</span>
                </Show>
              </span>
              <span class="preview-row-meta">{p.cmd ?? `bound to ${p.bind}`}</span>
            </span>
            <Show when={props.starting === p.port} fallback={<ChevronRight size={16} class="preview-chevron" />}>
              <RefreshCw size={16} class="refresh-spinning" />
            </Show>
          </button>
        )}
      </For>
    </div>
  );
}

/** The forward viewer. `embedded` (a side-pane tab, Z-104): no back arrow and no Back/Esc handler — the tab's ✕ closes it. */
export function ForwardViewer(props: { forward: ForwardInfo; hostName: string; onClose: () => void; onStop: () => void; embedded?: boolean }) {
  return <Viewer {...props} />;
}

function Viewer(props: { forward: ForwardInfo; hostName: string; onClose: () => void; onStop: () => void; embedded?: boolean }) {
  const [src, setSrc] = createSignal(props.forward.url);
  const [address, setAddress] = createSignal(urlPath(props.forward.url));
  const [generation, setGeneration] = createSignal(0);
  const [live, setLive] = createSignal<ForwardInfo | undefined>(props.forward);
  const [stopped, setStopped] = createSignal(false);
  const [loading, setLoading] = createSignal(true);

  createEffect(() => {
    if (!props.embedded) onCleanup(pushBack(() => props.onClose()));
  });

  // Watch the forward for failed connections (port closed / PC gone).
  // This computer's own ports (`local-…`) have no forward to watch.
  const timer = setInterval(async () => {
    if (props.forward.id.startsWith('local-')) return;
    const result = await forwardList();
    if (!result.ok) return;
    const mine = result.value.find((f) => f.id === props.forward.id);
    setLive(mine);
    setStopped(!mine);
  }, POLL_MS);
  onCleanup(() => clearInterval(timer));

  const banner = () => viewerBanner(live(), stopped());
  // UNI-224: the system browser (never the app's own WebView); a build that can't copies the link.
  const [note, setNote] = createSignal<string | undefined>();
  const openOutside = async () => {
    const url = src();
    const text = openExternal(url) ? undefined : (await copyText(url)) ? 'Link copied — paste it in your browser' : 'Could not open the browser';
    setNote(text);
    if (text) setTimeout(() => setNote((n) => (n === text ? undefined : n)), 2500);
  };
  const reload = () => {
    setLoading(true);
    setGeneration((g) => g + 1);
  };
  const go = (event: Event) => {
    event.preventDefault();
    const next = previewUrl(props.forward.localPort, address());
    setAddress(urlPath(next));
    setSrc(next);
    reload();
  };

  return (
    <div class="preview-viewer" data-testid="preview-viewer" data-embedded={props.embedded ? '' : undefined}>
      <header class="preview-viewer-head detail-head">
        <Show when={!props.embedded}>
          <button type="button" class="ghost-icon-button" aria-label="Back to ports" onClick={props.onClose}>
            <ArrowLeft size={20} />
          </button>
        </Show>
        <form class="preview-urlbar" onSubmit={go}>
          <span class="preview-url-origin" title={`${props.hostName} port ${props.forward.remotePort}`}>
            {props.hostName}:{props.forward.remotePort}
          </span>
          <input
            class="preview-url-input"
            aria-label="Address"
            value={address()}
            autocapitalize="off"
            autocorrect="off"
            spellcheck={false}
            enterkeyhint="go"
            onInput={(e) => setAddress(e.currentTarget.value)}
          />
        </form>
        <button type="button" class="ghost-icon-button" aria-label="Reload" onClick={reload}>
          <RotateCw size={18} class={loading() ? 'refresh-spinning' : ''} />
        </button>
        <button type="button" class="ghost-icon-button" aria-label="Open in browser" onClick={() => void openOutside()}>
          <ExternalLink size={18} />
        </button>
        <button type="button" class="ghost-icon-button preview-stop" aria-label="Stop preview" onClick={props.onStop}>
          <Square size={16} />
        </button>
      </header>
      <Show when={banner()}>
        {(b) => (
          <div class="preview-banner" data-kind={b().kind} role="alert" data-testid="preview-banner">
            <TriangleAlert size={16} />
            <span class="preview-banner-text">
              {b().message}
              <Show when={b().kind === 'closed'}> Start it again on the PC, then reload.</Show>
              <Show when={b().kind === 'lost' && !stopped()}> Reload to reconnect.</Show>
              <Show when={stopped()}>{props.embedded ? ' Open it again from + › Local ports.' : ' Go back to the ports to start it again.'}</Show>
            </span>
            <Show
              when={!stopped()}
              fallback={
                <button type="button" class="preview-banner-btn" onClick={props.onClose}>
                  {props.embedded ? 'Close' : 'Ports'}
                </button>
              }
            >
              <button type="button" class="preview-banner-btn" onClick={reload}>
                Reload
              </button>
            </Show>
          </div>
        )}
      </Show>
      <Show when={note()}>
        <div class="preview-note" role="status">{note()}</div>
      </Show>
      <div class="preview-frame-wrap">
        <Show keyed when={`${generation()}|${src()}`}>
          <iframe
            class="preview-frame"
            title={`Preview of port ${props.forward.remotePort}`}
            src={src()}
            allow="clipboard-read; clipboard-write; fullscreen"
            // UNI-224: no allow-top-navigation — a frame-busting page (or a
            // `target=_top` link) used to replace the WHOLE app with the
            // previewed site, leaving no header, no ✕ and a dead Android Back.
            sandbox={PREVIEW_SANDBOX}
            onLoad={() => setLoading(false)}
          />
        </Show>
      </div>
    </div>
  );
}
