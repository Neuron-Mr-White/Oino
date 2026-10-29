import { it,expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { parseHostLink } from './link';
const fixtures=JSON.parse(readFileSync('../../protocol/fixtures/pairlink.json','utf8')) as {name:string;input:string;ok:boolean;path?:string[]}[];
for(const fixture of fixtures)it(fixture.name,()=>{const result=parseHostLink(fixture.input);expect(result.ok).toBe(fixture.ok);if(!result.ok)expect(result.error.path).toEqual(fixture.path);});
