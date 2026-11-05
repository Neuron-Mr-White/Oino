/**
 * Lifetime (WP-A24, D-20; H9's commands): what closing the window does
 * (`closeAction`: ask / minimize to tray / quit) and "Start at login" (hidden
 * in the tray). Desktop only — a phone has no tray.
 */
import { Show, createSignal, onCleanup, onMount } from 'solid-js';
import { LogOut, Settings2 } from 'lucide-solid';
import { z } from 'zod';
import { call } from '../../lib/ipc';
import { Row, Section, Segmented } from './rows';
import { SwitchField } from '../../components/Switch';
import { useAutostart } from '../../components/LifecycleUi';

type CloseAction = 'ask' | 'tray' | 'quit';

const CloseActionSchema = z.enum(['ask', 'tray', 'quit']);

export function LifetimeSection() {
  const [closeAction, setCloseAction] = createSignal<CloseAction>('ask');
  const [error, setError] = createSignal<string | undefined>();
  const autostart = useAutostart();

  const refresh = async () => {
    const result = await call('close_action_state', undefined, CloseActionSchema);
    if (result.ok) setCloseAction(result.value);
  };
  onMount(() => void refresh());
  onCleanup(() => undefined);

  const pick = async (next: CloseAction) => {
    setCloseAction(next);
    const result = await call('close_action_set', { action: next }, CloseActionSchema);
    if (!result.ok) {
      setError(result.error.message);
      void refresh();
      return;
    }
    setError(undefined);
  };

  return (
    <Section title="General" id="sec-general" icon={<Settings2 size={14} />}>
      <Row
        label="Close button"
        description="Ask, minimize to the tray, or quit — phones and automations keep working while the tray lives"
        icon={<LogOut size={18} />}
      >
        <Segmented
          label="Close button"
          value={closeAction()}
          options={['ask', 'tray', 'quit'] as const}
          labels={{ ask: 'Ask', tray: 'Tray', quit: 'Quit' }}
          onChange={(v) => void pick(v)}
        />
      </Row>
      <Show when={error()}>
        <p class="px-4 pb-3 text-xs text-shell" role="alert">{error()}</p>
      </Show>
      <Row label="Start at login" description="Starts hidden in the tray; the app then adopts or spawns the host">
        <SwitchField label="Start at login" checked={autostart.on()} onChange={(v) => void autostart.set(v)} testId="autostart" />
      </Row>
    </Section>
  );
}
