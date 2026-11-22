import { describe, expect, it } from 'vitest';
import { isGeneralWorkspaceCwd } from './generalWorkspace';

describe('isGeneralWorkspaceCwd', () => {
  it('keeps home and ordinary app scratch paths under the general workspace rule', () => {
    expect(isGeneralWorkspaceCwd('/home/you')).toBe(true);
    expect(isGeneralWorkspaceCwd('/home/you/.local/share/oino/scratch/chat')).toBe(true);
  });

  it('folds UUID and bare hex-id project folders', () => {
    expect(isGeneralWorkspaceCwd('/work/.od-projects/06f493f8-cd36-4e13-a012-123456789abc')).toBe(true);
    expect(isGeneralWorkspaceCwd('/work/jobs/11494a503f24')).toBe(true);
    expect(isGeneralWorkspaceCwd('/work/my-project')).toBe(false);
  });

  it('folds known hidden, test, and temporary scratch parents', () => {
    expect(isGeneralWorkspaceCwd('/work/.od-projects/project')).toBe(true);
    expect(isGeneralWorkspaceCwd('/work/pytest-run/jobs/current')).toBe(true);
    expect(isGeneralWorkspaceCwd('/tmp/project')).toBe(true);
    expect(isGeneralWorkspaceCwd('/tmp')).toBe(true);
  });

  it('keeps a tmp folder inside a real project as that project', () => {
    expect(isGeneralWorkspaceCwd('/home/you/code/app/tmp')).toBe(false);
    expect(isGeneralWorkspaceCwd('/work/tmp/project')).toBe(false);
    expect(isGeneralWorkspaceCwd('/tmpfiles/project')).toBe(false);
  });
});
