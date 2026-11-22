import { describe, expect, it, vi } from 'vitest';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
import { graphMermaid, nodeDot, nodeRunLabel, parseVisualize, parseVisualizeParams, shortProfile, statusClass, workOfAgent } from './LhGraph';

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

describe('[graph-view] per-node profile + model', () => {
  const tree = {
    t: 'Layout',
    children: [
      { t: 'Code', language: 'mermaid', source: 'flowchart TD' },
      { t: 'Markdown', source: '- w1 [running] Survey the API\n  - ran on rel-qa · zai/glm-5.3 · thinking high\n- w2 [queued] Write the client' },
    ],
  };

  it('reads the indented ran-on line of a current tree (older apps ignore it)', () => {
    expect(parseVisualize(tree).nodes).toEqual([
      { id: 'w1', status: 'running', instruction: 'Survey the API', profile: 'rel-qa', model: 'zai/glm-5.3', thinking: 'high' },
      { id: 'w2', status: 'queued', instruction: 'Write the client' },
    ]);
  });

  it('prefers the structured params nodes', () => {
    const params = {
      v: 1,
      nodes: [
        { id: 'w1', status: 'completed', instruction: 'Survey', profile: 'subagent_explore', model: 'zai/glm-5.3', thinking: 'low', inputs: [] },
        { id: 'w2', status: 'blocked', instruction: 'Client', profile: 'subagent_general', inputs: ['r-w1'] },
        { id: 7, status: 'x' },
      ],
    };
    const v = parseVisualize(tree, params);
    expect(v.structured).toBe(true);
    expect(v.nodes.map((n) => [n.id, n.status, n.profile, n.model])).toEqual([
      ['w1', 'completed', 'subagent_explore', 'zai/glm-5.3'],
      ['w2', 'blocked', 'subagent_general', undefined],
    ]);
    expect(parseVisualizeParams({})).toBeUndefined();
  });

  it('labels a node with its short profile, pretty model and thinking', () => {
    expect(shortProfile('subagent_explore')).toBe('explore');
    expect(shortProfile('rel-qa')).toBe('rel-qa');
    expect(nodeRunLabel({ profile: 'rel-qa', model: 'zai/glm-5.3', thinking: 'high' })).toBe('rel-qa · GLM 5.3 · high');
    expect(nodeRunLabel({ profile: 'subagent_general' })).toBe('general');
    expect(nodeRunLabel({})).toBe('');
    expect(nodeRunLabel({ profile: 'rel-qa', model: 'zai/glm-5.3', toolCalls: 12 })).toBe('rel-qa · GLM 5.3 · 12 tool calls');
  });

  it('builds the app mermaid: run labels (clipped), edges, supersession and status classes', () => {
    const src = graphMermaid(
      [
        { id: 'a', status: 'completed', instruction: '', profile: 'rel-qa', model: 'zai/glm-5.3', inputs: [] },
        { id: 'b-1', status: 'running', instruction: '', profile: 'subagent_general', model: 'cc/claude-opus-5-5', inputs: ['r-a'] },
        { id: 'c', status: 'failed', instruction: '', inputs: ['r-a'] },
        { id: 'd', status: 'superseded', instruction: '', supersededBy: 'b-1' },
      ],
      16,
    );
    expect(src).toContain('n_a["a · completed<br/>rel-qa · GLM 5.3"]');
    expect(src).toContain('n_b_1["b-1 · running<br/>general · Claud…"]');
    expect(src).toContain('n_a --> n_b_1');
    expect(src).toContain('n_d -.-> n_b_1');
    expect(src).toContain('class n_b_1 running');
    expect(src).toContain('class n_c failed');
    expect(src).toContain('class n_d off');
  });

  it('styles failed / blocked / superseded and gives running a spinner state', () => {
    expect([statusClass('running'), statusClass('failed'), statusClass('blocked'), statusClass('superseded'), statusClass('stopped'), statusClass('queued')]).toEqual([
      'running',
      'failed',
      'blocked',
      'off',
      'off',
      'queued',
    ]);
    expect(nodeDot('blocked')).toBe('blocked');
  });
});
