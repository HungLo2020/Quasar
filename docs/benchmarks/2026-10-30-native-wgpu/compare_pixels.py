import json,os,re,subprocess,tempfile
from pathlib import Path
from PIL import Image
out=Path(tempfile.mkdtemp(prefix='quasar-wgpu-final-pixels-'));root=Path(__file__).resolve().parents[3]
env=dict(os.environ,WGPU_BACKEND='vulkan',XDG_CONFIG_HOME=str(out/'config'))
rows=[]
for tick,args in [(1,[]),(30,[]),(120,[]),(30,['--no-shadows']),(30,['--no-normal-maps']),(30,['--no-instancing','--no-culling'])]:
 row={'tick':tick,'options':args};images=[];hashes=[]
 for kind,binary in [('baseline',Path(os.environ.get('QUASAR_BASELINE_BIN', '/tmp/quasar-wgpu-baseline-bin'))/'nightwatch'),('native',root/'target/release/nightwatch')]:
  path=out/f'{len(rows)}-{kind}.png';command=[str(binary),'--headless','--autoplay','--no-audio','--seed','42','--ticks',str(tick),'--resolution','640x360','--asset-root',str(root/'examples/nightwatch/assets'),'--screenshot',str(path),*args]
  result=subprocess.run(command,cwd=root,env=env,capture_output=True,text=True,timeout=90);assert result.returncode==0,(command,result.stderr)
  path.with_suffix('.log').write_text(result.stdout+result.stderr);images.append(Image.open(path).convert('RGBA').tobytes());hashes.append(re.search(r'hash=([a-f0-9]+)',result.stdout).group(1))
 differences=[abs(a-b) for a,b in zip(*images)];row.update(differing_channels=sum(x>0 for x in differences),max_channel_difference=max(differences),hashes=hashes);assert hashes[0]==hashes[1],row;assert row['max_channel_difference']==0,row;rows.append(row);print(row,flush=True)
(out/'summary.json').write_text(json.dumps(rows,indent=2)+'\n');print('artifacts='+str(out),flush=True)
