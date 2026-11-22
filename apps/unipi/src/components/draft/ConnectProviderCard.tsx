/**
 * D-01: no provider on this PC can run a model yet. A calm card above the
 * composer says so and opens Settings › Providers — the first message would
 * otherwise start a chat that fails on its first turn.
 */
import { Plug } from 'lucide-solid';
import { openSettingsAt } from '../../lib/settings/open';

export function ConnectProviderCard() {
  return (
    <div class="connect-card" role="status" data-testid="connect-provider-card">
      <span class="connect-card-ic" aria-hidden="true">
        <Plug size={16} />
      </span>
      <span class="connect-card-text">
        <span class="connect-card-title">Connect a provider to start</span>
        <span class="connect-card-body">No model provider on this PC has a working key or sign-in yet.</span>
      </span>
      <button type="button" class="btn btn-secondary connect-card-btn" onClick={() => openSettingsAt('providers', 'sec-providers')} data-testid="connect-provider-open">
        Connect a provider
      </button>
    </div>
  );
}
