/** Restrict subagent records to the current session. Unattributed legacy records are
 * accepted only from the current session's feed or service reply. */
export function scopedSubagents(value: unknown, sessionId?: string): unknown[] | undefined {
  const envelope = value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
  const list = Array.isArray(value) ? value : envelope?.records;
  if (!Array.isArray(list)) return undefined;
  const owner = envelope?.parentSessionId ?? envelope?.rootSessionId;
  if (typeof owner === 'string' && owner && sessionId && owner !== sessionId) return [];
  return list.filter((item) => {
    if (!item || typeof item !== 'object') return false;
    const record = item as Record<string, unknown>;
    const parent = record.parentSessionId ?? record.rootSessionId;
    return !(typeof parent === 'string' && parent && sessionId && parent !== sessionId);
  });
}
