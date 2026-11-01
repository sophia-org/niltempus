"""One-shot control protocol and saved-record checks; no capture or timing claim."""
import importlib.util
import json
import os
from pathlib import Path
import re
import time


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name+'.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


h = load('hold_sophia')
q = load('qualify')
KEYS = {'schema', 'status', 'kind', 'connection_epoch', 'content_grant_epoch',
        'output', 'shell_output_generation', 'nonce'}


def ready(value, kind):
    if type(value) is not dict or value.keys() != KEYS or kind not in ('delay', 'invalid'):
        raise ValueError('one-shot READY shape')
    for key in KEYS-{'status','kind','nonce'}:
        h.r.uint(value[key], True)
    if (value['schema'] != 1 or value['status'] != 'control_ready' or value['kind'] != kind
            or type(value['nonce']) is not str or not re.fullmatch('[0-9a-f]{32}',value['nonce'])):
        raise ValueError('one-shot READY identity')
    return value


def qualify(log, kind, value):
    ready(value,kind)
    lines = log.decode().splitlines()
    hints = [line for line in lines if 'gpu_control ' in line]
    if len(hints) != 1 or not hints[0].startswith('gpu_control schema=1 status=endpoint_hint shell_socket='):
        raise ValueError('one-shot peer missing or restarted')
    if any(tag in line for line in lines for tag in
           ('gpu_colour_hold', 'gpu_invalid_import_failed', 'gpu_release_delay_failed', 'gpu_bar_profile')):
        raise ValueError('foreign or failed diagnostic')
    modes = [line for line in lines if 'gpu_bar_dma_mode ' in line]
    if modes != ['gpu_bar_dma_mode schema=1 mode=fresh']:
        raise ValueError('one-shot DMA mode differs')
    delay_modes = [line for line in lines if 'gpu_release_delay_mode ' in line]
    expected_mode = ['gpu_release_delay_mode schema=1 frames=1 delay_usec=100000 timed=0'] if kind=='delay' else []
    if delay_modes != expected_mode:
        raise ValueError('one-shot declaration differs')
    records = [h.r.record(line.encode()) for line in lines if line.startswith('{')]
    events = ['start','upload_offer'] + (['upload_accepted','permit','offer','terminal','end'] if kind=='delay' else [])
    if [row.get('event') for row in records] != events or any(type(row.get('schema')) is not int or row['schema']!=2 for row in records):
        raise ValueError('one-shot peer journal differs')
    start = dict(schema=2,event='start',frames=1,wire='files',probe_bytes=0,source='dma_buf')
    if records[0] != start or any(type(records[0][k]) is not type(v) for k,v in start.items()):
        raise ValueError('one-shot start differs')
    joined = {key:value[key] for key in ('connection_epoch','content_grant_epoch','output')}
    upload = records[1].get('upload')
    fixed = dict(**joined,candidate_generation=1,resource_id=1,resource_generation=1,
                 width=1920,height=24,total_bytes=184320)
    if (records[1].keys() != {'schema','event','upload'} or type(upload) is not dict
            or upload.keys()!=fixed.keys()|{'begin_usec'} or any(upload.get(k)!=v for k,v in fixed.items())):
        raise ValueError('one-shot upload identity differs')
    for v in upload.values(): h.r.uint(v,True)
    delay = [line for line in lines if 'gpu_release_delay ' in line]
    invalid = [line for line in lines if 'gpu_invalid_import' in line]
    if kind=='invalid':
        if delay or len(invalid)!=2:
            raise ValueError('one-shot refusal records missing or duplicated')
        prefixes = ('gpu_invalid_import ', 'gpu_invalid_import_refused ')
        if any(not line.startswith(prefix) for line,prefix in zip(invalid,prefixes,strict=True)):
            raise ValueError('one-shot refusal order differs')
        first,last = [q.fields(line[len(prefix):]) for line,prefix in zip(invalid,prefixes,strict=True)]
        identity = dict(schema='1',transport='sophia',connection_epoch=str(value['connection_epoch']),
                        content_grant_epoch=str(value['content_grant_epoch']),resource='1',generation='1')
        if (first.keys()!=identity.keys()|{'status','offer','bytes'}
                or last.keys()!=identity.keys()|{'offer','status','reason','released'}
                or any(row[k]!=v for row in (first,last) for k,v in identity.items())
                or first['status']!='injected' or not q.uint(first['offer'])
                or not 184320<=q.uint(first['bytes'])<=2**63-1
                or last['offer']!=first['offer'] or last['status']!='3'
                or last['reason'] not in ('3','9') or last['released']!='0'):
            raise ValueError('not an exact matched Session refusal')
        return dict(status='consumer_import_refused',offer=q.uint(first['offer']),reason=q.uint(last['reason']),
                    bytes=q.uint(first['bytes']),producer_release=False,
                    limits='Remote Malformed or RendererFailed; Malformed alone does not prove a GPU import attempt.')
    if invalid:
        raise ValueError('import refusal in release control')
    proof = q.qualify_release_delay('\n'.join(delay)+'\n','sophia')
    if any(proof['identity'][key]!=value[key] for key in ('connection_epoch','content_grant_epoch')):
        raise ValueError('release trace grant differs from authenticated peer')
    accepted,permit,offer,terminal,end = records[2:]
    shapes = [
        (accepted,dict(schema=2,event='upload_accepted',resource_id=1,resource_generation=1),{'accepted_usec'}),
        (permit,dict(schema=2,event='permit',generation=1),{'demand_usec','received_usec'}),
        (offer,dict(schema=2,event='offer',**joined,candidate_generation=1),{'submitted_monotonic_usec'}),
        (terminal,dict(schema=2,event='terminal',**joined,candidate_generation=1,outcome='presented',kind=2,reason=0),{'observed_monotonic_usec'}),
        (end,dict(schema=2,event='end',offered=1,completed=1),set()),
    ]
    for row,fixed,times in shapes:
        if row.keys()!=fixed.keys()|times or any(row[k]!=v or type(row[k]) is not type(v) for k,v in fixed.items()):
            raise ValueError('one-shot completion shape differs')
        for key in times: h.r.uint(row[key],True)
    times = [upload['begin_usec'],proof['held'],proof['resume'],proof['released'],accepted['accepted_usec'],
             permit['demand_usec'],permit['received_usec'],offer['submitted_monotonic_usec'],terminal['observed_monotonic_usec']]
    if times!=sorted(times):
        raise ValueError('release and presentation clock order differs')
    return proof


def drive(connection, peer, expected, evidence):
    kind = expected['control']
    value = ready(h.line(connection,peer,time.monotonic()+30),kind)
    peer.check()
    before,_ = h.c.log_bytes(expected['session_log'])
    if any(line.startswith(b'{') or b'gpu_invalid_import ' in line or b'gpu_release_delay ' in line
           for line in before.splitlines()):
        raise ValueError('one-shot work began before authenticated ACK')
    (evidence/'before-ack.log').write_bytes(before)
    ack = dict(schema=1,status='ack',generation=1,nonce=value['nonce'])
    data = (json.dumps(ack,separators=(',',':'))+'\n').encode()
    if connection.send(data)!=len(data):
        raise ValueError('partial one-shot ACK; no retry')
    done = h.line(connection,peer,time.monotonic()+30)
    if (type(done) is not dict or done!=dict(schema=1,status='control_done',kind=kind)
            or type(done['schema']) is not int):
        raise ValueError('one-shot DONE differs')
    # DONE and stdout travel on separate streams; wait for the bounded log drain.
    directory = h.c.private_directory(str(Path(expected['session_log']).parent))
    notification = h.c.watch(directory)
    try:
        until = time.monotonic()+5
        while True:
            peer.check()
            log,_ = h.c.log_bytes(expected['session_log'])
            rows = log.splitlines()
            json_count = sum(line.startswith(b'{') for line in rows)
            control_count = sum((b'gpu_invalid_import' if kind=='invalid' else b'gpu_release_delay ') in line for line in rows)
            if json_count >= (2 if kind=='invalid' else 7) and control_count >= (2 if kind=='invalid' else 3):
                proof = qualify(log,kind,value)
                break
            h.wait([notification,peer.fd],peer.session,until)
            try: os.read(notification,65536)
            except BlockingIOError: pass
    finally:
        os.close(notification); os.close(directory)
    (evidence/'completed.log').write_bytes(log)
    return dict(schema=1,status='control_verified',control=kind,ready=value,ack=ack,done=done,proof=proof,
                peer_pid=peer.pid,peer_starttime=peer.starttime,supervisor_pid=peer.supervisor,
                supervisor_starttime=peer.supervisor_starttime,
                peer_executable=dict(expected['peer'],dev=peer.peer_identity[0],ino=peer.peer_identity[1]),
                supervisor_executable=dict(expected['supervisor'],dev=peer.supervisor_identity[0],ino=peer.supervisor_identity[1]),
                limits='Authenticated same-user peer; no capture or performance claim. Peer exit code is not independently observed; wrapper stops Session after the remote result.')
