import { Show, createSignal } from 'solid-js';
import { useSettingsApi } from '../settings/context';
import { ErrorBanner } from './ErrorBanner';
export function SettingsNotices() {
  const api = useSettingsApi();
  const [dismissed, setDismissed] = createSignal(false);
  const [details, setDetails] = createSignal(false);
  return <div class="space-y-2">
    <Show when={api.resetKeys().length && !dismissed()}>
      <aside role="status" class="reset-banner">
        <div class="flex items-center gap-3"><span class="flex-1">{api.resetKeys().length} settings were reset to defaults</span><button onClick={() => setDetails(!details())}>Details</button><button aria-label="Dismiss settings notice" onClick={() => setDismissed(true)}>×</button></div>
        <Show when={details()}><p class="text-text-dim text-xs mt-2">{api.resetKeys().join(', ')}</p></Show>
      </aside>
    </Show>
    <Show when={api.newerVersionNotice()}><p class="text-xs text-text-muted">Settings from a newer app version.</p></Show>
    <Show when={api.error()}>{error => <ErrorBanner error={error()} dismiss={api.dismissNotice}/>}</Show>
  </div>;
}
