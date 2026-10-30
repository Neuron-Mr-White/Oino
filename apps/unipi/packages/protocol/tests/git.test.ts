import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import {
  ControlSchema, GitBranchesDataSchema, GitDiffDataSchema, GitLogDataSchema, GitRepoDataSchema,
  GitShowDataSchema, GitStatusDataSchema, GitSyncDataSchema, GitShortstatDataSchema,
  GitTurnFilesDataSchema, PROTO_VERSION,
} from '../src/index.js';
import { atLeast } from './semver.js';

type Fixture = { name: string; schema: string; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/git.json', import.meta.url), 'utf8'));
const schemas: Record<string, z.ZodTypeAny> = {
  Control: ControlSchema,
  GitRepoData: GitRepoDataSchema,
  GitStatusData: GitStatusDataSchema,
  GitDiffData: GitDiffDataSchema,
  GitLogData: GitLogDataSchema,
  GitShowData: GitShowDataSchema,
  GitBranchesData: GitBranchesDataSchema,
  GitSyncData: GitSyncDataSchema,
  GitShortstatData: GitShortstatDataSchema,
  GitTurnFilesData: GitTurnFilesDataSchema,
};

describe('git contract fixtures (UNI-217)', () => {
  it('proto is 1.10.0+ (git)', () => {
    expect(atLeast(PROTO_VERSION, '1.10.0')).toBe(true);
    expect(atLeast('1.10.0', '1.9.0')).toBe(true);
    expect(atLeast('1.9.0', '1.10.0')).toBe(false);
  });
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const schema = schemas[fixture.schema];
      expect(schema, fixture.schema).toBeDefined();
      expect(schema!.safeParse(fixture.value).success).toBe(fixture.ok);
    });
  }
});
