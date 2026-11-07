import http.server, json, os, pathlib, subprocess, threading, urllib.request, hashlib
root = pathlib.Path('/home/oi/Projects/Personal/llm/oino')
out = pathlib.Path('/tmp/oino-v3-integrate-r3/recorded-background')
for sub in ['agent', 'home', 'work']:
    (out/sub).mkdir(parents=True, exist_ok=True)
(out/'work/notes.txt').write_text('alpha\nbeta\n')
import shutil
stage = out/'dist'
stage.mkdir(exist_ok=True)
for package in ['oino.openai-compat', 'unipi.background-tasks']:
    shutil.copytree(root/'extensions/dist'/package, stage/package, dirs_exist_ok=True)
(out/'agent/settings.json').write_text(json.dumps({'autonomy':{'defaultLevel':'high'}}))
requests = []
class Proxy(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def do_POST(self):
        body = self.rfile.read(int(self.headers['Content-Length']))
        requests.append(json.loads(body))
        req = urllib.request.Request('https://openrouter.ai/api/v1/chat/completions', data=body, headers={'Authorization':'Bearer '+os.environ['OPENROUTER_API_KEY'], 'Content-Type':'application/json'})
        try:
            with urllib.request.urlopen(req, timeout=90) as response:
                data = response.read()
                self.send_response(200)
                self.send_header('Content-Type', response.headers.get('Content-Type'))
                self.end_headers()
                self.wfile.write(data)
        except Exception as error:
            self.send_error(502, str(error))
server = http.server.ThreadingHTTPServer(('127.0.0.1',0), Proxy)
threading.Thread(target=server.serve_forever,daemon=True).start()
(out/'agent/models.json').write_text(json.dumps({'providers':{'openrouter':{'baseUrl':f'http://localhost:{server.server_port}/v1','api':'openai-completions','models':[{'id':'openai/gpt-4.1-mini','contextWindow':128000,'maxTokens':4096}]}}}))
env = dict(os.environ, OINO_DIST_EXTENSIONS=str(stage), HOME=str(out/'home'), OINO_AGENT_DIR=str(out/'agent'))
cmd = [str(root/'target/release/oino'),'-nc','-ns','-np','--offline','--api-key','local-proxy','--model','openrouter/openai/gpt-4.1-mini','--mode','json','-p',f'In this isolated integration directory, use read on {out}/work/notes.txt, grep for alpha in that file, edit alpha to gamma, then bash pwd. All four tools are required. Also run bash command echo background-ok with summary background smoke, riskLevel low and fireAndForget true. Use task_output with its returned task_id and block true to get its completed output. End with done.']
with open(out/'events.jsonl','w') as stdout, open(out/'stderr.log','w') as stderr:
    result = subprocess.run(cmd,cwd=out/'work',env=env,stdout=stdout,stderr=stderr,timeout=150)
server.shutdown()
(out/'requests.json').write_text(json.dumps(requests,indent=2)+'\n')
canonical = lambda value: json.dumps(value,ensure_ascii=False,separators=(',',':')).encode()
prefixes = [canonical({'tools':r.get('tools'), 'messages':r['messages'][:2]}) for r in requests]
events = [json.loads(line) for line in (out/'events.jsonl').read_text().splitlines()]
tools = [{'tool':e.get('toolName'),'error':e.get('isError')} for e in events if e.get('type')=='tool_execution_end']
summary = {'exit':result.returncode,'request_count':len(requests),'prefix_bytes':len(prefixes[0]) if prefixes else 0,'prefix_sha256':[hashlib.sha256(p).hexdigest() for p in prefixes], 'prefix_identical':bool(prefixes) and all(p==prefixes[0] for p in prefixes),'tool_results':tools,'file_text':(out/'work/notes.txt').read_text()}
(out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
assert result.returncode==0 and summary['prefix_identical']
assert {'read','grep','edit','bash','task_output'} <= {t['tool'] for t in tools}
assert not any(t['error'] for t in tools)
assert summary['file_text']=='gamma\nbeta\n'
