import { describe, expect, it, vi } from 'vitest';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
import { nodeDot, parseVisualize, workOfAgent } from './LhGraph';

describe('P1-04: graph inspector data', () => {
  it('reads the mermaid source and node lines of /visualize-progress', () => {
    const tree = {
      t: 'Layout',
      children: [
        { t: 'Code', language: 'mermaid', source: 'flowchart TD\n  w1["w1 · running"]' },
        { t: 'Markdown', source: '- w1 [running] Survey the API\n- w2 [queued] Write the client\nGoal: ship — active (turn 1/9)' },
      ],
    };
    expect(parseVisualize(tree)).toEqual({
      mermaid: 'flowchart TD\n  w1["w1 · running"]',
      nodes: [
        { id: 'w1', status: 'running', instruction: 'Survey the API' },
        { id: 'w2', status: 'queued', instruction: 'Write the client' },
      ],
    });
    expect(parseVisualize(undefined)).toEqual({ nodes: [] });
  });

  it('maps statuses to dots and worker titles to work ids', () => {
    expect([nodeDot('running'), nodeDot('queued'), nodeDot('completed'), nodeDot('failed'), nodeDot('superseded')]).toEqual(['running', 'queued', 'done', 'failed', 'off']);
    expect(workOfAgent('Graph · w1')).toBe('w1');
    expect(workOfAgent('Swarm · item-3')).toBe('item-3');
    expect(workOfAgent('Explore subagent · w1')).toBeUndefined();
  });
});
