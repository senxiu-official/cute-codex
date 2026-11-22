#!/usr/bin/env python3
"""Private real app-server + loopback mock-model contract test (stdlib only).

Usage: python private_owner_input_batch.py APP_SERVER ABS_PRIVATE_CASE
The caller must provide an owner-private directory (0700 / Windows private ACL).
No live home, provider credentials, service mutation, or machine sandbox setup.
"""
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import queue
import secrets
import socket
import struct
import subprocess
import sys
import threading
import time
import zlib

exe, root = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
assert exe.is_file() and root.is_absolute()
root.mkdir(mode=0o700, parents=True, exist_ok=True)
assert not (root / 'config.toml').exists(), 'use a new private case'
os.chmod(root, 0o700)
(root / '.cutex-test-private-home').write_text('native-upstream-62 owner batch test\n')
requests = []
model_release = threading.Event()
model_release.set()
class Model(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def do_GET(self):
        self.send_error(404)
    def do_POST(self):
        data = self.rfile.read(int(self.headers['Content-Length']))
        assert not self.headers.get('Content-Encoding'), 'disable request compression'
        requests.append(json.loads(data))
        assert model_release.wait(90), 'private model release timed out'
        events = [
            {'type':'response.created','response':{'id':'r'+str(len(requests))}},
            {'type':'response.output_item.done','item':{'type':'message','role':'assistant','id':'answer','content':[{'type':'output_text','text':'done'}]}},
            {'type':'response.completed','response':{'id':'r'+str(len(requests)), 'usage':{'input_tokens':1,'output_tokens':1,'total_tokens':2}}},
        ]
        body = ''.join('event: '+e['type']+'\ndata: '+json.dumps(e)+'\n\n' for e in events).encode()
        self.send_response(200); self.send_header('Content-Type','text/event-stream')
        self.send_header('Content-Length',str(len(body))); self.end_headers(); self.wfile.write(body)
model = http.server.ThreadingHTTPServer(('127.0.0.1',0), Model)
threading.Thread(target=model.serve_forever, daemon=True).start()
(root / 'config.toml').write_text(f'''model = "gpt-5.1"
model_provider = "private"
approval_policy = "never"
sandbox_mode = "read-only"
[features]
enable_request_compression = false
hooks = true
[model_providers.private]
name = "Private mock model"
base_url = "http://127.0.0.1:{model.server_port}/v1"
wire_api = "responses"
requires_openai_auth = false
''')
# Every private prompt hook logs its execution; only the explicit fixture prompt
# is blocked. This also proves preflighted hooks are not run again by the turn.
hook=root/'owner_hook.py'
hook.write_text("import json,sys\nfrom pathlib import Path\np=json.load(sys.stdin)\n"
                +"with Path("+repr(str(root/'hook-runs.jsonl'))+").open('a') as f:f.write(json.dumps(p)+'\\n')\n"
                +"if p.get('prompt')=='BLOCK_PRIVATE_BATCH':print(json.dumps({'decision':'block','reason':'private batch blocked'}))\n")
# Private fixture paths are whitespace-free: bare executable syntax works in
# both PowerShell (where a quoted executable needs &) and POSIX shells.
assert not any(ch.isspace() for ch in sys.executable+str(hook))
(root/'hooks.json').write_text(json.dumps({'hooks':{'UserPromptSubmit':[{'hooks':[{
    'type':'command','command':sys.executable+' '+str(hook)}]}]}}))
env = {k:v for k,v in os.environ.items() if k.upper() in ('SYSTEMROOT','WINDIR','PATH','PATHEXT','COMSPEC','PROGRAMDATA','PROGRAMFILES','PROGRAMFILES(X86)')}
for k in ('CODEX_HOME','HOME','USERPROFILE','TEMP','TMP','TMPDIR','APPDATA','LOCALAPPDATA'):
    env[k] = str(root)
env['RUST_LOG'] = 'error'
env['NO_PROXY'] = '127.0.0.1,localhost'
processes = []
def launch(args):
    log = (root / ('server-'+str(len(processes))+'.log')).open('wb')
    p = subprocess.Popen([str(exe),'--disable-plugin-startup-tasks-for-tests',*args],cwd=root,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log)
    processes.append((p, log))
    return p
def stop(p):
    # Popen retains the exact child handle; never traverse or kill a process tree.
    if p.poll() is None:
        p.stdin.close()
        try: p.wait(timeout=4)
        except subprocess.TimeoutExpired:
            p.terminate(); p.wait(timeout=10)

class RPC:
    def __init__(self, send, receive):
        self.send, self.receive, self.seq = send, receive, 0
        self.messages = queue.Queue()
        self.events = []
        def read():
            try:
                while True: self.messages.put(json.loads(receive()))
            except Exception as e: self.messages.put(e)
        threading.Thread(target=read,daemon=True).start()
    def next(self):
        message = self.messages.get(timeout=60)
        if isinstance(message,Exception): raise message
        return message
    def call(self,method,params,allow_error=False):
        self.seq += 1
        self.send(json.dumps({'jsonrpc':'2.0','id':self.seq,'method':method,'params':params}).encode())
        while True:
            msg = self.next()
            if msg.get('id') == self.seq:
                if not allow_error: assert 'error' not in msg, msg
                return msg
            self.events.append(msg)
    def initialize(self):
        value = self.call('initialize',{'clientInfo':{'name':'native-batch-private-test','version':'1'},'capabilities':{'experimentalApi':True}})['result']
        self.send(b'{"jsonrpc":"2.0","method":"initialized"}')
        return value
    def completed(self,turn_id):
        while True:
            for index,event in enumerate(self.events):
                if event.get('method') == 'turn/completed' and event['params']['turn']['id'] == turn_id:
                    self.events.pop(index); assert event['params']['turn']['status'] == 'completed', event; return
            self.events.append(self.next())

def stdio(p):
    def send(data): p.stdin.write(data+b'\n'); p.stdin.flush()
    return RPC(send,p.stdout.readline)

def websocket(address,token=None):
    end = time.monotonic()+30
    while True:
        try:
            if isinstance(address,tuple): s = socket.create_connection(address,timeout=2)
            else:
                s = socket.socket(socket.AF_UNIX); s.settimeout(2); s.connect(str(address))
            break
        except OSError:
            if time.monotonic()>end: raise
            time.sleep(.05)
    s.settimeout(90)
    key = base64.b64encode(secrets.token_bytes(16)).decode()
    auth = f'Authorization: Bearer {token}\r\n' if token else ''
    s.sendall((f'GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n{auth}\r\n').encode())
    header = b''
    while not header.endswith(b'\r\n\r\n'): header += s.recv(1)
    assert b' 101 ' in header, header
    def exact(n):
        data=b''
        while len(data)<n:
            part=s.recv(n-len(data))
            if not part: raise EOFError('socket closed')
            data+=part
        return data
    def frame(data,opcode=1):
        length=len(data); mask=secrets.token_bytes(4)
        size=bytes([0x80|length]) if length<126 else (b'\xfe'+struct.pack('!H',length) if length<65536 else b'\xff'+struct.pack('!Q',length))
        s.sendall(bytes([0x80|opcode])+size+mask+bytes(byte^mask[i%4] for i,byte in enumerate(data)))
    def read():
        output=b''
        while True:
            first,second=exact(2); size=second&127
            if size==126: size=struct.unpack('!H',exact(2))[0]
            if size==127: size=struct.unpack('!Q',exact(8))[0]
            mask=exact(4) if second&128 else None
            data=exact(size)
            if mask: data=bytes(byte^mask[i%4] for i,byte in enumerate(data))
            opcode=first&15
            if opcode==8: raise EOFError('websocket closed')
            if opcode==9: frame(data,10); continue
            if opcode==10: continue
            output+=data
            if first&128:return output
    return RPC(frame,read),s

def bound(thread_id):
    binding=root/'binding.json'
    binding.write_text(json.dumps({'version':1,'ownerId':'native-batch-private','threadId':thread_id,'runtimeGeneration':7}))
    os.chmod(binding,0o600)
    if os.name=='nt':
        sock=socket.socket();sock.bind(('127.0.0.1',0));port=sock.getsockname()[1];sock.close()
        token=secrets.token_hex(32)
        p=launch(['--listen',f'ws://127.0.0.1:{port}','--external-input-binding-file',str(binding),'--ws-auth','capability-token','--ws-token-sha256',hashlib.sha256(token.encode()).hexdigest()])
        rpc,s=websocket(('127.0.0.1',port),token)
    else:
        endpoint=root/'a.sock'
        p=launch(['--listen','unix://'+str(endpoint),'--external-input-binding-file',str(binding)])
        rpc,s=websocket(endpoint)
    assert rpc.initialize()['ownerInputBatch']==1
    rpc.call('thread/resume',{'threadId':thread_id})
    return p,rpc,s

def text(value):return [{'type':'text','text':value}]
def turn(rpc,thread_id,id,content,prefix=None):
    params={'threadId':thread_id,'clientUserMessageId':id,'input':content}
    if prefix is not None:params['ownerInputs']=prefix
    return rpc.call('turn/start',params)
def reject(rpc,params):
    before=len(requests)
    result=rpc.call('turn/start',params,True)
    assert 'error' in result,result
    assert len(requests)==before,'refused batch sampled'
    return result

report={'binary':str(exe),'sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'checks':[]}
try:
    p=launch([]); rpc=stdio(p)
    assert rpc.initialize().get('ownerInputBatch') is None
    listed=rpc.call('hooks/list',{'cwds':[str(root)]})['result']['data']
    hooks=[item for entry in listed for item in entry['hooks']]
    assert len(hooks)==1,listed
    # Trust only the test hook's exact advertised hash in this private home.
    stop(p)
    with (root/'config.toml').open('a') as config:
        for item in hooks:
            config.write('\n[hooks.state.'+json.dumps(item['key'])+']\ntrusted_hash = '
                         +json.dumps(item['currentHash'])+'\n')
    p=launch([]);rpc=stdio(p);rpc.initialize()
    thread_id=rpc.call('thread/start',{'cwd':str(root),'model':'gpt-5.1','approvalPolicy':'never'})['result']['thread']['id']
    reject(rpc,{'threadId':thread_id,'clientUserMessageId':'unsupported','input':text('unsupported'),'ownerInputs':[]})
    report['checks'].append('unbound-empty-prefix-refused-before-model')
    seed=turn(rpc,thread_id,'already-read',text('seed'))
    rpc.completed(seed['result']['turn']['id']);stop(p)
    p,rpc,s=bound(thread_id)
    model_release.clear()
    before_busy=len(requests)
    busy=turn(rpc,thread_id,'busy-existing',text('keep active'))
    try:
        deadline=time.monotonic()+60
        while len(requests)==before_busy:
            assert time.monotonic()<deadline,'active fixture did not reach model'
            time.sleep(.02)
        not_idle=reject(rpc,{'threadId':thread_id,'clientUserMessageId':'busy-new',
                            'input':text('must wait'),'ownerInputs':[]})
        assert not_idle['error']['data']=={'ownerInputBatchNotSubmitted':'not_idle'},not_idle
    finally:
        model_release.set()
    rpc.completed(busy['result']['turn']['id'])
    report['checks'].append('active-turn-batch-definite-not-idle-no-steering')
    blocked={'threadId':thread_id,'clientUserMessageId':'blocked-carrier','input':text('tail'),
             'ownerInputs':[{'clientUserMessageId':'blocked-prefix','input':text('BLOCK_PRIVATE_BATCH')}]}
    before=len(requests)
    denied=reject(rpc,blocked)
    assert denied['error']['data']=={'ownerInputBatchNotSubmitted':'hook_blocked'},denied
    assert len(requests)==before
    # Remove the blocking content, retaining both original IDs: no admission
    # intent may have sealed a batch that was refused by preflight.
    blocked['ownerInputs'][0]['input']=text('unblocked-original-id')
    accepted=rpc.call('turn/start',blocked);rpc.completed(accepted['result']['turn']['id'])
    runs=[json.loads(line) for line in (root/'hook-runs.jsonl').read_text().splitlines()]
    assert sum(item.get('prompt')=='unblocked-original-id' for item in runs)==1,runs
    assert sum(item.get('prompt')=='tail' for item in runs)==1,runs
    report['checks'].append('hook-blocked-before-ack-no-intent-same-ids-reusable-hooks-once')
    prefix=[{'clientUserMessageId':'a','input':text('/literal/path')},{'clientUserMessageId':'b','input':text('second')}]
    params={'threadId':thread_id,'clientUserMessageId':'draft','input':text('draft'),'ownerInputs':prefix}
    invalid=dict(params); invalid['ownerInputs']=[{'clientUserMessageId':'already-read','input':text('old')},prefix[0]]
    reject(rpc,invalid);report['checks'].append('partial-already-read-refused-whole')
    invalid=dict(params);invalid['ownerInputs']=[prefix[0],prefix[0]]
    reject(rpc,invalid);report['checks'].append('duplicate-refused-before-turn')
    invalid=dict(params);invalid['threadId']='00000000-0000-4000-8000-000000000000'
    reject(rpc,invalid);report['checks'].append('cross-binding-refused')
    before=len(requests)
    result=rpc.call('turn/start',params);turn_id=result['result']['turn']['id'];rpc.completed(turn_id)
    assert len(requests)==before+1
    owner=[item for item in requests[-1]['input'] if item.get('role')=='user']
    contents=[[part.get('text') for part in item['content'] if part.get('type')=='input_text'] for item in owner]
    assert contents[-3:]==[['/literal/path'],['second'],['draft']],contents
    history=rpc.call('thread/items/list',{'threadId':thread_id,'turnId':turn_id,'limit':100})['result']
    (root/'history.json').write_text(json.dumps(history,indent=2))
    items=[entry.get('item',entry) for entry in history['data']]
    ids=[item.get('clientUserMessageId',item.get('clientId')) for item in items if item.get('type')=='userMessage']
    assert ids==['a','b','draft'],ids
    report['checks'].append('one-model-request-ordered-original-history-ids')
    refusal=reject(rpc,params)
    assert refusal['error']['code']==-32600 and not (refusal['error'].get('data') or {}).get('outcomeUnknown'),refusal
    report['checks'].append('same-generation-replay-refused')
    s.close();stop(p)
    p,rpc,s=bound(thread_id)
    reject(rpc,params);report['checks'].append('restart-replay-refused')
    invalid=dict(params);invalid['clientUserMessageId']='new-carrier';invalid['ownerInputs']=[prefix[0]]
    reject(rpc,invalid);report['checks'].append('restart-partial-replay-refused')
    # Explicit [] remains the new contract with a fresh ordinary carrier.
    result=turn(rpc,thread_id,'empty-prefix',text('empty-prefix'),[]);rpc.completed(result['result']['turn']['id'])
    report['checks'].append('empty-prefix-supported')
    # No-draft carrier is the last original owner; even an image-only carrier
    # must retain literal slash/path prefix text and produce one request.
    def chunk(kind,data):return struct.pack('!I',len(data))+kind+data+struct.pack('!I',zlib.crc32(kind+data))
    image=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!IIBBBBB',1,1,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(b'\x00\xff\x00\x00'))+chunk(b'IEND',b'')
    png='data:image/png;base64,'+base64.b64encode(image).decode()
    before=len(requests)
    image_result=turn(rpc,thread_id,'image-last',[{'type':'image','url':png}],
                      [{'clientUserMessageId':'path-first','input':text('/tmp/literal-owner-path')}])
    rpc.completed(image_result['result']['turn']['id'])
    assert len(requests)==before+1
    owner=[item for item in requests[-1]['input'] if item.get('role')=='user']
    assert any(part.get('text')=='/tmp/literal-owner-path' for part in owner[-2]['content']),owner[-2:]
    assert any(part.get('type')=='input_image' for part in owner[-1]['content']),owner[-1]
    report['checks'].append('no-draft-image-only-carrier-literal-prefix')
    # Deliberately discard the final RPC response: only query original IDs after
    # reopening, and verify an explicit direct replay cannot sample again.
    lost={'threadId':thread_id,'clientUserMessageId':'lost-carrier','input':text('lost-carrier'),
          'ownerInputs':[{'clientUserMessageId':'lost-prefix','input':text('lost-prefix')}]}
    before=len(requests);rpc.seq+=1
    rpc.send(json.dumps({'jsonrpc':'2.0','id':rpc.seq,'method':'turn/start','params':lost}).encode())
    deadline=time.monotonic()+60
    while len(requests)==before:
        assert time.monotonic()<deadline,'unknown-response request did not reach model'
        time.sleep(.02)
    # The received result is intentionally never inspected or used for retry.
    s.close();stop(p)
    p,rpc,s=bound(thread_id)
    reject(rpc,lost)
    report['checks'].append('discarded-response-restart-does-not-resend')
    # Inject incomplete history only in this private case, after resume. This
    # exercises the real Core Fatal -> JSON-RPC data boundary without failpoints.
    rollouts=list(root.rglob('rollout-*'+thread_id+'*.jsonl'))
    assert len(rollouts)==1,rollouts
    rollout=rollouts[0]
    original=rollout.read_bytes()
    try:
        with rollout.open('ab') as handle:handle.write(b'\n{invalid-private-history\n')
        unknown=reject(rpc,{'threadId':thread_id,'clientUserMessageId':'unknown-carrier',
                            'input':text('unknown'),'ownerInputs':[]})
        assert unknown['error']['code']==-32600,unknown
        assert unknown['error']['data']=={'ownerInputBatchNotSubmitted':'history_unavailable'},unknown
        report['checks'].append('history-failure-before-admission-definite-not-submitted-no-model')
    finally:
        s.close();stop(p)
        rollout.write_bytes(original)
    report['status']='PASS';report['modelRequests']=len(requests)
finally:
    model_release.set()
    for p,log in processes:stop(p);log.close()
    model.shutdown();model.server_close()
    (root/'RESULT.json').write_text(json.dumps(report,indent=2))
    (root/'model-requests.json').write_text(json.dumps(requests,indent=2))
    if 'rpc' in globals():(root/'rpc-events.json').write_text(json.dumps(rpc.events,indent=2))
print(json.dumps(report))
