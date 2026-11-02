/**
 * Which model providers on a PC can actually run a model (D-01, D-37).
 *
 * The host's `auth_providers` rows say whether a credential exists
 * (`configured`); a host that also checks it answers `ready` (the key works)
 * and an optional `reason` (why not). Both pages read it through here, so
 * "Ready" never rests on an environment variable alone when the host can say
 * more. No host answer → the UI treats a configured provider as usable (it
 * cannot know better) and says nothing it cannot back.
 */
import { z } from 'zod';
import { hostCmd } from '../../settings/hostCmd';

export const ProviderSchema = z.object({
  id: z.string(),
  name: z.string().default(''),
  oauth: z.boolean().default(false),
  configured: z.boolean().default(false),
  stored: z.string().optional(),
  env: z.array(z.string()).default([]),
  /** The host checked the credential: `true` it works, `false` it was rejected. */
  ready: z.boolean().optional(),
  /** Why a configured provider is not ready ("the key was rejected", "offline"…). */
  reason: z.string().optional(),
});
export type Provider = z.infer<typeof ProviderSchema>;

export const ProvidersSchema = z.object({ providers: z.array(ProviderSchema) });

/** Whether the host (or, failing that, the credential store) says this provider can run a model. */
export const providerUsable = (p: Provider): boolean => (p.ready !== undefined ? p.ready : p.configured);

export type Readiness = {
  /** `false` when the host could not be asked. */
  known: boolean;
  providers: Provider[];
  /** At least one provider can run a model. */
  any: boolean;
  /** This provider can (unknown providers: yes — nothing says otherwise). */
  providerReady: (id: string) => boolean;
};

export function readinessOf(providers: Provider[] | undefined): Readiness {
  if (!providers) return { known: false, providers: [], any: true, providerReady: () => true };
  const byId = new Map(providers.map((p) => [p.id, p]));
  return {
    known: true,
    providers,
    any: providers.some(providerUsable),
    providerReady: (id) => {
      const p = byId.get(id);
      // A provider the host's credential list doesn't know was registered by an extension
      // (its key lives with the extension): nothing says it can't run, so it can.
      return p ? providerUsable(p) : true;
    },
  };
}

/**
 * Readiness that also counts extension-registered providers: the host's `auth_providers`
 * rows list only built-in providers, so a PC whose only working models come from an
 * extension (a private router, a gateway bridge) would read as "no provider". Any listed
 * model whose provider the host doesn't list — or the model the user picked — proves a
 * usable provider.
 */
export function withModelProviders(
  r: Readiness | undefined,
  providerIds: ReadonlyArray<string | undefined>,
): Readiness | undefined {
  if (!r?.known || r.any) return r;
  const listed = new Set(r.providers.map((p) => p.id));
  const extension = providerIds.some((id) => !!id && !listed.has(id));
  return extension ? { ...r, any: true } : r;
}

/** Asks the host which providers work (a failed ask is "unknown", never "none"). */
export async function hostReadiness(hostId: string, opts: { validate?: boolean } = {}): Promise<Readiness> {
  // G-05: `validate` waits for the host's real credential check (bounded) — a saved dead key is never "ready".
  const res = await hostCmd({ kind: 'auth_providers', ...(opts.validate ? { validate: true } : {}) }, ProvidersSchema, hostId).catch(() => undefined);
  return readinessOf(res?.ok ? res.value.providers : undefined);
}
