import type { AppError } from '@unipi/protocol';
export type ParsedLink = { ticket: string; code: string; name?: string };

function validTicket(ticket: string): boolean {
  if (!ticket.startsWith('endpoint')) return false;
  const alphabet='ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  let bits=0, accumulator=0; const bytes:number[]=[];
  for (const char of ticket.slice(8).toUpperCase()) {
    const value=alphabet.indexOf(char); if(value<0)return false;
    accumulator=(accumulator<<5)|value; bits+=5;
    if(bits>=8){bits-=8;bytes.push((accumulator>>bits)&255);accumulator&=(1<<bits)-1;}
  }
  if(accumulator!==0||bytes.length<34||bytes[0]!==0)return false;
  let offset=33;
  const integer=()=>{let value=0,shift=0;for(let i=0;i<5;i++){const byte=bytes[offset++];if(byte===undefined)return -1;value+=(byte&127)*2**shift;if(byte<128)return value;shift+=7;}return -1;};
  const count=integer();if(count<0||count>100)return false;
  for(let i=0;i<count;i++){
    const kind=integer();
    if(kind===0){const length=integer();if(length<0||offset+length>bytes.length)return false;try{new URL(new TextDecoder().decode(new Uint8Array(bytes.slice(offset,offset+length))));}catch{return false;}offset+=length;}
    else if(kind===1){const family=integer();if(family!==0&&family!==1)return false;offset+=family===0?4:16;if(integer()<0)return false;}
    else return false;
  }
  return offset===bytes.length;
}
export function parseHostLink(raw:string):{ok:true;value:ParsedLink}|{ok:false;error:AppError}{
 const invalid=(field:string,message:string)=>({ok:false as const,error:{code:'E_PAIRING_INVALID' as const,message,path:[field],retryable:false}});
 const trimmed=raw.trim();let ticket:string|undefined,code:string|undefined,name:string|undefined;
 if(/^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)){
  // PH-36: a regex on the raw string, never `new URL().hostname` — WebView ≤ 113
  // leaves the host of a non-special scheme (unipi://) empty.
  const m=/^unipi:\/\/pair\/?(?:\?([^#]*))?(?:#.*)?$/i.exec(trimmed);
  if(!m)return invalid('t',"This isn't a Unipi pairing link");
  let params:URLSearchParams;try{params=new URLSearchParams(m[1]??'');}catch{return invalid('t',"This isn't a Unipi pairing link");}
  ticket=params.get('t')??undefined;code=params.get('c')??undefined;name=params.get('n')??undefined;
 }else{const fields=trimmed.split(/\s+/);if(fields.length!==2)return invalid(fields.length===1?'c':'t',"This isn't a Unipi pairing link");[ticket,code]=fields;}
 if(!ticket)return invalid('t','Missing ticket');
 if(!code)return invalid('c','Missing code');
 if(!validTicket(ticket))return invalid('t','Invalid endpoint ticket');
 if(!/^[0123456789ABCDEFGHJKMNPQRSTVWXYZ]{10}$/.test(code))return invalid('c','Code must be 10 Crockford base32 characters');
 return {ok:true,value:{ticket,code,...(name?{name}: {})}};
}
export function shortHostId(id:string,len=8):string{return id.slice(0,len);}
