import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import {
  ControlSchema,
  CronJobSchema,
  CronJobsDataSchema,
  CronLogDataSchema,
  CronParseDataSchema,
  CronRunStartedDataSchema,
  CronRunSchema,
  CronRunsDataSchema,
  CronSettingsDataSchema,
  CronShowDataSchema,
  CronStatusDataSchema,
  CronStoppedDataSchema,
  PROTO_VERSION,
} from '../src/index.js';
import { atLeast } from './semver.js';

type Schema =
  | 'Control'
  | 'CronJob'
  | 'CronJobsData'
  | 'CronShowData'
  | 'CronRun'
  | 'CronRunsData'
  | 'CronLogData'
  | 'CronRunStartedData'
  | 'CronStoppedData'
  | 'CronStatusData'
  | 'CronParseData'
  | 'CronSettingsData';
type Fixture = { name: string; schema: Schema; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/cron.json', import.meta.url), 'utf8'));
const schemas: Record<Schema, z.ZodTypeAny> = {
  Control: ControlSchema,
  CronJob: CronJobSchema,
  CronJobsData: CronJobsDataSchema,
  CronShowData: CronShowDataSchema,
  CronRun: CronRunSchema,
  CronRunsData: CronRunsDataSchema,
  CronLogData: CronLogDataSchema,
  CronRunStartedData: CronRunStartedDataSchema,
  CronStoppedData: CronStoppedDataSchema,
  CronStatusData: CronStatusDataSchema,
  CronParseData: CronParseDataSchema,
  CronSettingsData: CronSettingsDataSchema,
};

describe('cron_* contract fixtures (WP-A23 over the H8 job-file wire)', () => {
  it('proto is 2.0.0 (v2: mux/board/desktop/chat layer removed)', () => expect(PROTO_VERSION).toBe('2.0.0'));
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const result = schemas[fixture.schema].safeParse(fixture.value);
      expect(result.success).toBe(fixture.ok);
    });
  }
  it('every reply DTO has at least one fixture (the shapes stay pinned)', () => {
    const covered = new Set(fixtures.map((f) => f.schema));
    for (const name of Object.keys(schemas)) {
      if (name === 'Control') continue;
      expect(covered, `${name} needs a fixture`).toContain(name);
    }
  });
  it('a wake link carries the session file/id, never a live pid (H8 job files)', () => {
    const job = CronJobSchema.parse(
      fixtures.find((f) => f.name.includes('session-file wake link'))!.value,
    );
    expect(job.wake?.session_file).toBeTruthy();
    expect(job.wake?.pid).toBeUndefined();
  });
  it('the fired run answers its session path (the run id is the session id)', () => {
    const run = CronRunSchema.parse(fixtures.find((f) => f.name === 'CronRun with the fired session (the run id is the session id)')!.value);
    expect(run.session_path).toContain(run.id);
  });
  it('control fixtures speak the 2.0.0 proto', () => {
    const hellos = fixtures.filter((f) => f.schema === 'Control').map((f) => f.value as { proto: string });
    expect(hellos.length).toBeGreaterThan(10);
    expect(hellos.every((h) => atLeast(h.proto, '2.0.0'))).toBe(true);
  });
});
