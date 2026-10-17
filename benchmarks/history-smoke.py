#!/usr/bin/env python3
"""Isolated 64-child / 128-command regression check; not a capacity ceiling."""
import concurrent.futures, json, os, socket, subprocess, tempfile, time, uuid
from pathlib import Path
binary=str((Path(__file__).resolve().parent.parent/'target'/'debug'/'pmr'))
base={k:v for k,v in os.environ.items() if not k.startswith(('PMR_','PM2_'))}
with tempfile.TemporaryDirectory(prefix='pmr-scale-64-') as directory:
    env=dict(base,PMR_HOME=directory,PM2_HOME=directory,NO_COLOR='1')
    def cli(*args):
        result=subprocess.run([binary,*args],env=env,capture_output=True,text=True,timeout=30)
        assert result.returncode==0,result.stderr
        return result.stdout
    def rpc(request):
        with socket.socket(socket.AF_UNIX) as conn:
            conn.settimeout(30)
            conn.connect(directory+'/rpc.sock')
            conn.sendall((json.dumps(request)+'\n').encode())
            f=conn.makefile('rb')
            response=json.loads(f.readline())
            assert response['ok'],response
            return response['data']
    def submit(index,program,args):
        spec=dict(id=str(uuid.uuid4()),program=program,args=args,cwd=directory,
            name=f'command-{index}',namespace='smoke',env={})
        return rpc(dict(method='history',params=dict(request=dict(Submit=spec))))
    try:
        cli('ping')
        cli('limits','--running','64','--queued','256')
        started=time.monotonic()
        with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
            ids=list(pool.map(lambda n:submit(n,'/bin/sleep',['5']),range(64)))
        deadline=time.monotonic()+4
        while True:
            limits=json.loads(cli('limits'))
            if limits['running']==64:break
            assert time.monotonic()<deadline,limits
            time.sleep(.02)
        with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
            queued=list(pool.map(lambda n:submit(n,'/bin/echo',['done']),range(64,128)))
        peak=64
        while True:
            history=json.loads(cli('history','list','--limit','1000'))['records']
            limits=json.loads(cli('limits'))
            peak=max(peak,limits['running'])
            assert peak<=64,limits
            if len(history)==128 and all(r['status']=='succeeded' for r in history):break
            assert time.monotonic()-started<30,history[-1]
            time.sleep(.1)
        assert json.loads(cli('jlist'))==[]
        assert limits['active_commands']==0
        print(json.dumps(dict(real_commands=len(history),peak_running=peak,
            elapsed_seconds=round(time.monotonic()-started,3),
            active_registry=0,retention=json.loads(cli('retention'))['settings'])),flush=True)
    finally:
        cli('kill')
