/**
 * Settings › Providers (WP-A25, G-15 `/login` + `/logout`): every model
 * provider with its credential state, a **Sign in** (OAuth) and **API key**
 * action, and **Sign out**.
 *
 * Where each action goes:
 * - list: `host_cmd › auth_providers` (WP-A24's hub-level command), so the
 *   page works **without** a chat;
 * - sign in: `_oino/login` on an open chat — the OAuth flow raises its dialogs
 *   on that session's frontend (U10 renders them) and the authorization URL is
 *   opened in the system browser as soon as the provider hands it over;
 * - set / clear key: `host_cmd › set_api_key` / `logout` (pure credential-store
 *   writes, no session needed).
 *
 * R8 applies to Sign in only (a session frontend is required); the rest of the
 * page works chatless.
 */
import { For, Show, createResource, createSignal } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { KeyRound, Plug } from 'lucide-solid';
import { z } from 'zod';
import { LOCAL_HOST } from '../../lib/host/client';
import { openExternal } from '../../lib/board/share';
import { hostCmd } from '../../settings/hostCmd';
import { sessions } from '../../lib/session/store';
import { sessionUi } from '../../ui/registries';
import { manageChat } from '../../lib/settings/manage';
import { appNavigate } from '../../lib/nav';
import { ErrorBanner } from '../../components/ErrorBanner';
import type { AppError } from '@unipi/protocol';
import { ProviderSchema, ProvidersSchema, type Provider } from '../../lib/model/readiness';
import { Row, Section } from './rows';

export { ProviderSchema, type Provider };

/**
 * How one provider reads on the page (D-37): "Ready" only when the host
 * checked the credential; a key that is merely present reads "Key set", and a
 * rejected one says so — never "Ready" for a dead key.
 */
export function providerState(p: Provider): { label: string; detail?: string; bad?: boolean } {
  if (p.configured && p.ready === false) return { label: 'Not working', detail: p.reason ?? 'the provider rejected this credential', bad: true };
  const how = p.stored === 'oauth' ? 'OAuth' : p.stored === 'api_key' ? 'API key' : p.env.length ? `from ${p.env.join(' / ')}` : 'environment';
  if (p.configured && p.ready === true) return { label: p.stored === 'oauth' ? 'Signed in' : 'Ready', detail: how };
  if (p.stored === 'oauth') return { label: 'Signed in', detail: 'OAuth' };
  if (p.stored === 'api_key') return { label: 'API key saved' };
  if (p.configured) return { label: 'Key found', detail: `${how} · not checked yet` };
  return { label: 'Not configured' };
}

/** The first http(s) URL a provider's dialog asks the user to open (OAuth). */
export function oauthUrlIn(request: { [field: string]: unknown }): string | undefined {
  for (const key of ['url', 'message', 'title', 'placeholder', 'text']) {
    const value = request[key];
    if (typeof value !== 'string') continue;
    const match = /https?:\/\/[^\s"'<>]+/.exec(value);
    if (match) return match[0];
  }
  return undefined;
}

/** E-22: usable providers first, then configured-but-not-working, then the rest (names alphabetical inside a group). */
export function sortProviders(list: readonly Provider[]): Provider[] {
  const rank = (p: Provider) => (p.configured && p.ready !== false ? 0 : p.configured ? 1 : 2);
  return [...list].sort((a, b) => rank(a) - rank(b) || (a.name || a.id).localeCompare(b.name || b.id));
}

export function ProvidersPage() {
  const navigate = useNavigate();
  const [providers, { refetch }] = createResource(async (): Promise<Provider[]> => {
    const result = await hostCmd({ kind: 'auth_providers' }, ProvidersSchema, LOCAL_HOST);
    return result.ok ? result.value.providers : [];
  });
  const [error, setError] = createSignal<AppError | string | undefined>();
  const [busy, setBusy] = createSignal<string | undefined>();
  const [keyFor, setKeyFor] = createSignal<string | undefined>();
  const [key, setKey] = createSignal('');

  const hasChat = () => !!manageChat();

  const signIn = async (p: Provider) => {
    const chat = manageChat();
    if (!chat) {
      setError('Open a chat to sign in — the provider asks its questions there.');
      return;
    }
    const handle = sessions().peek(chat.hostId, chat.sessionId);
    if (!handle) {
      setError('Open a chat to sign in — the provider asks its questions there.');
      return;
    }
    setBusy(p.id);
    setError(undefined);
    try {
      await handle.cmd({ type: '_oino/login', provider: p.id });
      // The provider's dialogs arrive on the session's UI registry (U10 docks
      // them); a URL they carry belongs in the system browser — an OAuth page
      // must never render in the app webview.
      for (const dialog of sessionUi(chat.hostId, chat.sessionId).dialogs()) {
        const url = oauthUrlIn(dialog);
        if (url) {
          openExternal(url);
          break;
        }
      }
      appNavigate(`/s/${encodeURIComponent(chat.hostId)}/${encodeURIComponent(chat.sessionId)}`);
    } catch (e) {
      setError(String((e as { message?: unknown })?.message ?? e));
    } finally {
      setBusy(undefined);
    }
  };

  const saveKey = async (p: Provider) => {
    if (!key().trim()) return;
    setBusy(p.id);
    const result = await hostCmd({ kind: 'set_api_key', provider: p.id, key: key().trim() }, z.null(), LOCAL_HOST);
    setBusy(undefined);
    if (!result.ok) {
      setError(result.error);
      return;
    }
    setKey('');
    setKeyFor(undefined);
    setError(undefined);
    void refetch();
  };

  const signOut = async (p: Provider) => {
    setBusy(p.id);
    const result = await hostCmd({ kind: 'logout', provider: p.id }, z.null(), LOCAL_HOST);
    setBusy(undefined);
    if (!result.ok) {
      setError(result.error);
      return;
    }
    setError(undefined);
    void refetch();
  };

  return (
    <Section title="Providers" id="sec-providers" icon={<Plug size={14} />}>
      <p class="settings-page-blurb settings-provider-blurb">Model providers this host can reach, and how each one is signed in.</p>
      <Show when={error()} keyed>
        {(err) => (
          <div class="p-3">
            {typeof err === 'string' ? (
              <p class="settings-field-hint settings-field-hint-bad" role="alert" data-testid="providers-error">
                {err}
              </p>
            ) : (
              <ErrorBanner error={err} dismiss={() => setError(undefined)} />
            )}
          </div>
        )}
      </Show>
      <Show when={!providers.loading && (providers.latest ?? []).length === 0}>
        <Row label="No providers" description="Install a provider extension (openai-compat, anthropic, …) in Settings › Extensions" />
      </Show>
      <Show when={!providers.loading && (providers.latest ?? []).length > 0}>
        <p class="settings-provider-summary" data-testid="providers-summary">
          {(() => {
            const all = providers.latest ?? [];
            const ready = all.filter((p) => p.configured && p.ready !== false).length;
            const broken = all.filter((p) => p.configured && p.ready === false).length;
            return `${ready} ready${broken ? ` · ${broken} not working` : ''} · ${all.length - ready - broken} not set up`;
          })()}
        </p>
      </Show>
      <For each={sortProviders(providers.latest ?? [])}>
        {(p) => {
          const state = () => providerState(p);
          return (
            <>
              <div class="settings-provider-row" data-testid={`provider-${p.id}`}>
                <span class="min-w-0">
                  <span class="block text-[15px] text-text truncate">{p.name || p.id}</span>
                  <span class="block text-xs text-text-dim">
                    <span class="settings-provider-state" data-ok={p.configured && state().bad !== true ? '' : undefined} data-bad={state().bad ? '' : undefined}>
                      {state().label}
                    </span>
                    <Show when={state().detail}>{(detail) => <span> · {detail()}</span>}</Show>
                    <Show when={!p.configured && p.env.length > 0}>
                      <span> · set {p.env.join(' or ')}</span>
                    </Show>
                  </span>
                </span>
                <span class="settings-provider-actions">
                  <Show when={p.oauth}>
                    <button
                      type="button"
                      class="btn-secondary settings-btn"
                      data-testid={`provider-signin-${p.id}`}
                      disabled={busy() === p.id || !hasChat()}
                      title={!hasChat() ? 'Open a chat to sign in' : undefined}
                      onClick={() => void signIn(p)}
                    >
                      Sign in
                    </button>
                  </Show>
                  <button type="button" class="btn-secondary settings-btn" data-testid={`provider-key-${p.id}`} disabled={busy() === p.id} onClick={() => setKeyFor(keyFor() === p.id ? undefined : p.id)}>
                    API key
                  </button>
                  <Show when={p.configured && p.stored}>
                    <button type="button" class="btn-secondary settings-btn" data-testid={`provider-signout-${p.id}`} disabled={busy() === p.id} onClick={() => void signOut(p)}>
                      Sign out
                    </button>
                  </Show>
                </span>
              </div>
              <Show when={keyFor() === p.id}>
                <div class="settings-provider-key" data-testid={`provider-key-form-${p.id}`}>
                  <KeyRound size={15} class="text-text-dim shrink-0" />
                  <input
                    type="password"
                    class="settings-input"
                    aria-label={`API key for ${p.name || p.id}`}
                    placeholder="sk-…"
                    value={key()}
                    onInput={(e) => setKey(e.currentTarget.value)}
                    data-testid={`provider-key-input-${p.id}`}
                  />
                  <button type="button" class="btn-secondary settings-btn" disabled={!key().trim() || busy() === p.id} onClick={() => void saveKey(p)} data-testid={`provider-key-save-${p.id}`}>
                    Save
                  </button>
                </div>
              </Show>
            </>
          );
        }}
      </For>
      <Show when={!hasChat() && (providers.latest ?? []).some((p) => p.oauth)}>
        <div class="settings-manage-empty" data-testid="providers-nochat">
          <p class="text-xs text-text-dim">Signing in needs an open chat (the provider asks its questions there).</p>
          <button type="button" class="btn-secondary settings-btn" onClick={() => navigate(`/new/${encodeURIComponent(LOCAL_HOST)}`)} data-testid="providers-newchat">
            New chat here
          </button>
        </div>
      </Show>
    </Section>
  );
}
