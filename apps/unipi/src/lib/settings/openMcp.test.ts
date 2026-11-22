// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';

const nav = vi.hoisted(() => ({ to: [] as string[] }));
vi.mock('../nav', () => ({ appNavigate: (p: string) => nav.to.push(p) }));

const { openMcpSettings } = await import('./open');
const { projectScope, setProjectScope, settingsProjectCwd } = await import('../../settings/target');

afterEach(() => {
  setProjectScope(undefined);
  nav.to.length = 0;
});

describe('/mcp and /trust open Settings › MCP on the chat’s project (OIN-101)', () => {
  it('preselects the chat’s project and goes to the MCP page', () => {
    openMcpSettings('pc-1', '/home/you/projects/zm', 'servers');
    expect(nav.to).toEqual(['/settings/mcp']);
    expect(projectScope()).toEqual({ hostId: 'pc-1', cwd: '/home/you/projects/zm' });
    expect(settingsProjectCwd('pc-1')).toBe('/home/you/projects/zm');
  });

  it('a chat without a project opens the Global scope (clears an older project scope)', () => {
    setProjectScope({ hostId: 'pc-1', cwd: '/old' });
    openMcpSettings('pc-1', undefined, 'servers');
    expect(projectScope()).toBeUndefined();
    expect(nav.to).toEqual(['/settings/mcp']);
  });
});
