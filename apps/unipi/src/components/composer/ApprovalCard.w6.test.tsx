// @vitest-environment jsdom
/** w6-ui N5 (Always allow only when it can be saved), N8 (no "(legacy call)" title), N10 (ask_user shows the question). */
import { cleanup, render } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApprovalCard } from './ApprovalCard';
import type { ApprovalRequest } from '../../lib/session/autonomy';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
afterEach(cleanup);

const base: ApprovalRequest = { requestId: 'r', sessionId: 's', tool: 'bash', arguments: { command: 'echo hi' }, riskLevel: 'high', ceiling: 'medium', reason: 'needs approval' };
const show = (over: Partial<ApprovalRequest>) => render(() => <ApprovalCard onDecision={async () => {}} request={{ ...base, ...over }} />);
const buttons = (v: ReturnType<typeof show>) => [...v.container.querySelectorAll('.ap-btn')].map((b) => b.textContent);

describe('approval card', () => {
  it('N5: offers Always allow only when the host can save the rule, and says why otherwise', () => {
    expect(buttons(show({ alwaysAvailable: true }))).toEqual(['Allow once', 'Always allow', 'Deny']);
    cleanup();
    expect(buttons(show({}))).toEqual(['Allow once', 'Always allow', 'Deny']); // older host: flag absent
    cleanup();
    const v = show({ alwaysAvailable: false });
    expect(buttons(v)).toEqual(['Allow once', 'Deny']);
    expect(v.getByTestId('approval-no-always').textContent).toMatch(/isn’t trusted/);
  });

  it('N8: a bash call without a summary is titled by its command, not "(legacy call)"', () => {
    const v = show({ arguments: { command: 'git push origin main', summary: '(legacy call)' } });
    expect(v.container.textContent).not.toContain('legacy call');
    expect(v.container.querySelector('.dlg-question')?.textContent).toBe('Run: git push origin main');
  });

  it('N8: a real summary still titles the card', () => {
    const v = show({ arguments: { command: 'git push', summary: 'Push the branch' } });
    expect(v.container.querySelector('.dlg-question')?.textContent).toBe('Push the branch');
  });

  it('N10: an ask_user approval shows the question and options, not raw JSON', () => {
    const v = show({ tool: 'ask_user', riskLevel: null, arguments: { questions: [{ question: 'Which colour?', header: 'Colour', options: [{ label: 'Blue' }, { label: 'Red' }] }] } });
    expect(v.container.querySelector('.dlg-question')?.textContent).toBe('Question: Which colour?');
    const pre = v.container.querySelector('.ap-cmd')!.textContent!;
    expect(pre).toContain('Which colour?');
    expect(pre).toContain('Blue · Red');
    expect(pre).not.toContain('{');
  });

  it('G3-7: a write approval is titled "Write <file>" with a content preview, never the raw JSON', () => {
    const content = Array.from({ length: 2_000 }, (_, i) => `write line ${i}`).join('\n');
    const v = show({ tool: 'write', arguments: { path: 'out/big-live.txt', content } });
    expect(v.container.querySelector('.dlg-question')?.textContent).toBe('Write out/big-live.txt');
    const pre = v.container.querySelector('.ap-cmd')!;
    expect(pre.getAttribute('data-kind')).toBe('preview');
    expect(pre.textContent).toContain('write line 0');
    expect(pre.textContent).not.toContain('write line 1999');
    expect(pre.textContent).not.toContain('{"');
    expect(pre.textContent).toMatch(/more lines · [\d,]+ characters in all/);
    expect((pre.textContent ?? '').length).toBeLessThan(1_500);
    expect(v.queryByLabelText('Copy command')).toBeNull();
  });

  it('G3-7: an edit approval names the file and shows the new text', () => {
    const v = show({ tool: 'edit', arguments: { path: 'src/math.ts', edits: [{ oldText: 'a - b', newText: 'a + b' }] } });
    expect(v.container.querySelector('.dlg-question')?.textContent).toBe('Edit src/math.ts');
    expect(v.container.querySelector('.ap-cmd')!.textContent).toBe('a + b');
  });

  it('G3-11: the "Always allow" note speaks of the call for non-shell tools and of the command for shell ones', () => {
    const w = show({ tool: 'write', alwaysAvailable: false, arguments: { path: 'a.txt', content: 'x' } });
    expect(w.getByTestId('approval-no-always').textContent).toContain('this call can’t be saved as a rule');
    cleanup();
    const b = show({ alwaysAvailable: false, arguments: { command: 'git push' } });
    expect(b.getByTestId('approval-no-always').textContent).toContain('this command can’t be saved as a rule');
  });
});
