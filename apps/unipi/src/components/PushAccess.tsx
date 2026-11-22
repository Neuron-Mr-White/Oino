/**
 * Settings → "Files from your PCs" (UNI-248): per paired PC, whether it may
 * send files here (`push_list{wait:0}` → `enabled`), and when it's off the
 * same switch (this computer) / "Ask the PC" card as Preview — the push
 * "off" state, which used to be just a command in the docs.
 */
import { createSignal, For, onMount, Show, type JSX } from 'solid-js';
import { ChevronDown, Download, PowerOff, RefreshCw } from 'lucide-solid';
import { pushTransport } from '../lib/push';
import { FeatureOff } from './FeatureOff';
import { CommandBoxInline } from './CommandBoxInline';
import { LeaveSheet } from './LeaveSheet';

type HostState = 'loading' | 'on' | 'off' | 'offline';

export function PushAccess(props: { hosts: { id: string; name: string }[]; Section: (p: { title: string; icon: JSX.Element; children: JSX.Element }) => JSX.Element }) {
  const Section = props.Section;
  return (
    <Show when={props.hosts.length > 0}>
      <Section title="Files from your PCs" icon={<Download size={14} />}>
        <div data-testid="push-access">
          <For each={props.hosts}>{(h) => <HostPush host={h} />}</For>
        </div>
      </Section>
    </Show>
  );
}

function HostPush(props: { host: { id: string; name: string } }) {
  const [state, setState] = createSignal<HostState>('loading');
  const [open, setOpen] = createSignal(false);
  // UNI-253: a phone can turn off what it has.
  const [leaving, setLeaving] = createSignal(false);
  const check = async () => {
    setState('loading');
    try {
      const data = await pushTransport().list(props.host.id, 0);
      setState(data.enabled ? 'on' : 'off');
    } catch {
      setState('offline');
    }
  };
  onMount(() => void check());
  const text = () =>
    state() === 'on' ? 'Can send files here (unipi-host push)' : state() === 'off' ? 'Off — this PC can’t send files here' : state() === 'offline' ? 'Couldn’t reach this PC' : 'Checking…';
  return (
    <div data-testid={`push-access-${props.host.id}`} data-state={state()}>
      <div class="settings-row">
        <span class="settings-icon"><Download size={18} /></span>
        <div class="min-w-0">
          <p class="text-[15px] text-text truncate">{props.host.name}</p>
          <p class="text-xs text-text-dim mt-0.5">{text()}</p>
        </div>
        <div class="shrink-0 text-right text-text-muted">
          <Show when={state() === 'off'} fallback={
            <Show when={state() === 'loading'} fallback={
              <>
                <button type="button" class="ghost-icon-button" aria-label={`Check ${props.host.name} again`} onClick={() => void check()}><RefreshCw size={16} /></button>
                <button type="button" class="btn btn-secondary btn-sm" data-testid="push-leave" onClick={() => setLeaving(true)}>
                  <PowerOff size={14} /> Turn off
                </button>
              </>
            }>
              <RefreshCw size={16} class="refresh-spinning" />
            </Show>
          }>
            <button type="button" class="btn btn-secondary share-push-toggle" aria-expanded={open()} onClick={() => setOpen(!open())}>
              Turn on <ChevronDown size={15} class={open() ? 'share-fold-open' : ''} />
            </button>
          </Show>
        </div>
      </div>
      {/* UNI-253: removing this phone, or turning the feature off on the PC. */}
      <LeaveSheet open={leaving()} onClose={() => { setLeaving(false); void check(); }} hostId={props.host.id} feature="push" allowed={state() === 'on'} />
      <Show when={state() === 'off' && open()}>
        <div class="share-push-card">
          <FeatureOff feature="push" hostId={props.host.id} hostName={props.host.name} CommandBox={CommandBoxInline} onChanged={() => void check()} />
        </div>
      </Show>
    </div>
  );
}
