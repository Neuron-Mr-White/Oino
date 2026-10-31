# Minimal app.sock client (frame = [kind u8][len u32BE][json]); usage: python3 scripts/qa/host-probe.py <host dir>/app.sock
# Shows board_cli, host_cmd extensions, session_new and a prompt via {"type":"session_cmd","id":N,"cmd":{"type":"prompt","message":...}}.
import socket, struct, json, sys, time
SOCK=sys.argv[1]
def frame(kind, payload): return bytes([kind])+struct.pack('>I',len(payload))+payload
def hello(req): return frame(0, json.dumps({"type":"hello","proto":"2.1.0","req":req}).encode())
def readf(s, timeout=10):
    s.settimeout(timeout); h=b''
    while len(h)<5:
        c=s.recv(5-len(h))
        if not c: raise EOFError
        h+=c
    k=h[0]; n=struct.unpack('>I',h[1:])[0]; b=b''
    while len(b)<n: b+=s.recv(n-len(b))
    return k,b
def op(req):
    s=socket.socket(socket.AF_UNIX); s.connect(SOCK); s.sendall(hello(req)); k,b=readf(s,60); return s,json.loads(b)
_,r=op({"op":"board_cli","args":["status"]}); print("BOARD:", json.dumps(r)[:300])
_,r=op({"op":"host_cmd","cmd":{"kind":"extensions"}}); 
exts=(r.get("data") or {}).get("extensions") or []
print("EXTENSIONS:", len(exts), sorted(e.get("id") for e in exts)[:40] if exts else json.dumps(r)[:300])
s,r=op({"op":"session_new"}); print("SESSION_NEW:", r.get("type"), json.dumps(r)[:200])
if r.get("type")=="ok":
    s.sendall(frame(2, json.dumps({"type":"prompt","message":"Reply with exactly: host-ok"}).encode()))
    t=time.time(); text=''
    while time.time()-t<90:
        try: k,b=readf(s,30)
        except Exception as e: print("read end", e); break
        try: m=json.loads(b)
        except: continue
        blob=json.dumps(m)
        if 'host-ok' in blob: print("GOT host-ok in", m.get("type") or m.get("event",{}).get("type")); break
        if '"error' in blob.lower() and 'is_error": false' not in blob: print("ERR?", blob[:300])
