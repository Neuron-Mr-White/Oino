import { Plus, Server } from 'lucide-solid';
import { PathChip } from './ConnectionState';
import { For, createSignal, Show } from "solid-js";
import { useHosts } from "../hosts/context";
import { setAddHostOpen } from "./Pairing";
import { BoardSheet } from "./board/BoardSheet";
import { LauncherSheet } from "./chat/LauncherSheet";
import { can } from "../lib/platform";
export function HostsSettings() {
  const hosts = useHosts();
  const [target, setTarget] = createSignal("");
  const [launcherFor, setLauncherFor] = createSignal<{ id: string; name: string } | null>(null);
  return (
    <>
      <For each={hosts.hosts()}>
        {(host) => (
          <div class="settings-row flex-wrap"><span class="settings-icon"><Server size={18}/></span><div class="flex-1">
            <strong>{host.name}</strong>
            <p class="text-[13px] text-text-muted font-mono">{host.id.slice(0,8)}</p></div><PathChip path={hosts.paths()[host.id] || host.last_path}/>
            <Show when={can('chat')}>
              <button type="button" class="btn btn-ghost btn-sm" data-testid="host-launcher" onClick={() => setLauncherFor({ id: host.id, name: host.name })}>
                pi launcher
              </button>
            </Show>
            {/* UNI-231: "This computer" isn't a pairing — nothing to forget. */}
            <Show when={!(host.transport === 'local' && host.paired_at === 0)}>
              <button type="button" class="btn btn-ghost btn-sm" onClick={() => setTarget(host.id)}>
                Forget
              </button>
            </Show>
          </div>
        )}
      </For>
      <Show when={!hosts.hosts().length}>
        <p class="hosts-empty">No PC paired yet.</p>
      </Show>
      <button type="button" class="hosts-add-row" onClick={() => setAddHostOpen(true)}>
        <span class="settings-icon"><Plus size={18} /></span>
        Add a PC
      </button>
      <LauncherSheet pc={launcherFor()} onClose={() => setLauncherFor(null)} />
      <BoardSheet
        open={!!target()}
        onClose={() => setTarget("")}
        label="Forget this PC?"
        testId="forget-host-sheet"
        footer={
          <div class="dialog-actions">
            <button type="button" class="btn btn-secondary" onClick={() => setTarget("")}>Cancel</button>
            <button
              type="button"
              class="btn btn-primary"
              onClick={async () => {
                await hosts.forget(target());
                setTarget("");
              }}
            >
              Forget PC
            </button>
          </div>
        }
      >
        <p class="pair-intro">You will need to pair again to reconnect.</p>
      </BoardSheet>
    </>
  );
}
