import type { Lead, LeadMode } from './api';

export function autonomyLabel(level: string | undefined | null): string {
  return `Autonomy: ${level === 'low' ? 'Low' : level === 'high' ? 'High' : 'Medium'}`;
}

export const MODE_LABEL: Record<LeadMode, string> = { none: 'Plain', goal: 'Goal', ralph: 'Ralph', swarm: 'Swarm', graph: 'Graph' };
export const modeLabel = (mode: string | null | undefined) => (mode ? MODE_LABEL[mode as LeadMode] ?? mode : 'Mode');

/** `provider/model` → `model`; the provider is noise on a compact card. */
export const shortModel = (model: string | null | undefined) => (model ? model.slice(model.indexOf('/') + 1) : null);

export function elapsed(started: string | null | undefined, now: number): string | null {
  if (!started || !Number.isFinite(Date.parse(started))) return null;
  const minutes = Math.floor(Math.max(0, now - Date.parse(started)) / 60_000);
  if (minutes < 1) return '<1 min';
  return minutes < 60 ? `${minutes} min` : `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

export const money = (cost: number | null | undefined) => (cost === null || cost === undefined ? null : `$${cost.toFixed(2)}`);

export function progressText(lead: Lead): string | null {
  const p = lead.progress;
  if (!p) return null;
  return `${p.running} running · ${p.queued} queued · ${p.done} done`;
}
