import { describe, it, expect, vi } from 'vitest';
import { z } from 'zod';
import { invoke } from '@tauri-apps/api/core';
import { call } from './ipc';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
describe('typed IPC', () => {
 it('validates successful results', async () => { vi.mocked(invoke).mockResolvedValue({version:'1'}); expect(await call('app_info', {}, z.object({version:z.string()}))).toEqual({ok:true,value:{version:'1'}}); });
 it('returns schema errors without throwing', async () => { vi.mocked(invoke).mockResolvedValue(123); const result=await call('app_info',{},z.string()); expect(result.ok).toBe(false); if(!result.ok) expect(result.error.code).toBe('E_SCHEMA'); });
 it('parses command rejection', async () => { vi.mocked(invoke).mockRejectedValue({code:'E_SETTINGS',message:'Bad settings',retryable:false}); const result=await call('app_info',{},z.string()); expect(result.ok).toBe(false); if(!result.ok) expect(result.error.code).toBe('E_SETTINGS'); });
});
