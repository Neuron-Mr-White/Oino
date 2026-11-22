#!/usr/bin/env python3
"""Offline stdio MCP stand-in (newline-delimited JSON-RPC). argv[1]=server label; logs to $STDIO_LOG if set."""
import json, os, sys, time
label=sys.argv[1] if len(sys.argv)>1 else 'stdio'
# Owner-shaped Refero auth, using only a synthetic token. Do not log its value.
if '--header' in sys.argv:
    header = sys.argv[sys.argv.index('--header') + 1]
    if '$' in header or header != 'Authorization:' + os.environ.get('REFERO_AUTH', ''):
        sys.exit(2)
log=os.environ.get('STDIO_LOG')
def L(**k):
    if log:
        with open(log,'a') as f: f.write(json.dumps({'t':round(time.time(),3),'label':label,'cwd':os.getcwd(),**k})+'\n')
L(event='start', env_auth=('set' if os.environ.get('REFERO_AUTH') else 'unset'))
for line in sys.stdin:
    try: req=json.loads(line)
    except Exception: continue
    m=req.get('method'); L(method=m)
    if 'id' not in req: continue
    if m=='initialize': r={'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':f'offline {label}','version':'1'}}
    elif m=='tools/list': r={'tools':[{'name':'ping','description':f'{label} ping','inputSchema':{'type':'object','properties':{}}},{'name':'search_screens','description':f'{label} search','inputSchema':{'type':'object','properties':{}}}]}
    elif m=='tools/call': r={'content':[{'type':'text','text':json.dumps({'pong':label})}]}
    else: r={}
    sys.stdout.write(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':r})+'\n'); sys.stdout.flush()
