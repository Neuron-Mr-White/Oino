import { it,expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { parseHostLink } from './link';
const fixtures=JSON.parse(readFileSync('./protocol/fixtures/pairlink.json','utf8')) as {name:string;input:string;ok:boolean;path?:string[]}[];
for(const fixture of fixtures)it(fixture.name,()=>{const result=parseHostLink(fixture.input);expect(result.ok).toBe(fixture.ok);if(!result.ok)expect(result.error.path).toEqual(fixture.path);});
it('parses without URL hostname support (old WebViews, PH-36)',()=>{
 const good=fixtures.find((f)=>f.name==='name')!.input;
 const Orig=globalThis.URL;
 // WebView ≤ 113: non-special schemes parse with an empty hostname.
 class OldURL extends Orig{get hostname(){return this.protocol==='unipi:'?'':super.hostname;}}
 (globalThis as {URL:typeof URL}).URL=OldURL as typeof URL;
 try{const r=parseHostLink(good);expect(r.ok).toBe(true);if(r.ok)expect(r.value.name).toBeTruthy();}finally{(globalThis as {URL:typeof URL}).URL=Orig;}
});
it('says plainly when a paste is not a pairing link',()=>{const r=parseHostLink('https://example.com');expect(r.ok).toBe(false);if(!r.ok)expect(r.error.message).toBe("This isn't a Unipi pairing link");});
