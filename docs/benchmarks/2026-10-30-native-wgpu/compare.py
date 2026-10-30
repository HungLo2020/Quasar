import importlib.util,json,os,re,statistics,subprocess as S,tempfile,time
from pathlib import Path
root=Path(__file__).resolve().parents[3];out=Path(tempfile.mkdtemp(prefix='quasar-wgpu-performance-'));print('artifacts='+str(out),flush=True)
r,w=os.pipe();log=open(out/'xvfb.log','w');server=S.Popen([os.environ.get('XVFB', 'Xvfb'),'-displayfd',str(w),'-screen','0','1600x1000x24','-nolisten','tcp'],pass_fds=(w,),stdout=log,stderr=log);os.close(w);display=os.read(r,32).decode().strip();os.close(r);assert display
base=dict(os.environ,DISPLAY=':'+display,WINIT_UNIX_BACKEND='x11',XDG_CONFIG_HOME=str(out/'config'),WGPU_BACKEND='vulkan',WGPU_VALIDATION='0',WGPU_DEBUG='0',VULKANIC_BACKEND='vulkan',VULKANIC_GAL_VALIDATION='0',VULKANIC_VULKAN_VALIDATE='0');base.pop('WAYLAND_DISPLAY',None)
bins={'baseline':Path(os.environ.get('QUASAR_BASELINE_BIN', '/tmp/quasar-wgpu-baseline-bin')),'native':root/'target/release'}
try:
 spec_module=importlib.util.spec_from_file_location('quasar_benchmark_runs',root/'tools/benchmark_runs.py');module=importlib.util.module_from_spec(spec_module);spec_module.loader.exec_module(module)
 cases=[]
 for pair in range(3):
  for kind in (['baseline','native'] if pair%2==0 else ['native','baseline']):
   command=[str(bins[kind]/'nightwatch'),'--autoplay','--no-audio','--no-validation','--seed','42','--frames','664','--resolution','960x540','--benchmark-file','{output}','--asset-root',str(root/'examples/nightwatch/assets'),'--set','benchmark.warmup_frames=64','--set','benchmark.sample_capacity=600']
   cases.append({'name':f'{kind}-{pair}','command':command,'env':base})
 module.run_cases({'cases':cases},out/'nightwatch',1,120)
 for program in ['quasar-composition','framebuffer']:
  for policy in ['sync','poll']:
   for pair in range(3):
    for kind in (['baseline','native'] if pair%2==0 else ['native','baseline']):
     artifact=out/f'{program}-{policy}-{kind}-{pair}.json';command=[str(bins[kind]/program),'--frames','365' if program=='framebuffer' else '364','--benchmark',str(artifact)]
     if program=='quasar-composition' and policy=='sync':command+=['--sync']
     if program=='framebuffer' and policy=='poll':command+=['--poll']
     result=S.run(command,cwd=root,env=base,capture_output=True,text=True,timeout=120);artifact.with_suffix('.log').write_text(result.stdout+result.stderr);assert result.returncode==0,(command,result.stdout,result.stderr)
     report=json.loads(artifact.read_text());assert report['frames']==300,report['frames'];metric='cpu_present_ms'if program=='framebuffer'else'cpu_frame_ms';print(f'{program} {policy} {kind} {pair}: CPU p50={report[metric]["p50"]:.4f} ms GPU={report["gpu_frame_ms"]}',flush=True)
 report={'baseline_commit':'7a82ff3f23f0c6720dd1bcd0864be18ae8608b41','baseline_version':'0.2.19','native_version':'0.2.19 working tree','host':S.check_output(['uname','-srmo'],text=True).strip(),'rust':S.check_output(['rustc','--version'],text=True).strip(),'display':'X11 Xvfb 1600x1000; no compositor','driver_validation':False,'legacy_frontend_validation':False,'native_core_validation':'unmodified upstream defaults','profiling':'GPU timestamps enabled; native command interval excludes implicit queue upload copies','repetitions':3,'results':{}}
 def metrics(paths,program):
  samples=[json.loads(p.read_text())for p in paths];rows={}
  mapping={'cpu_frame_ms':'cpu_frame','cpu_submit_ms':'submit','gpu_commands_ms':'gpu_frame','cpu_prepare_ms':'renderer_prepare'} if program=='nightwatch' else {'cpu_frame_ms':'cpu_present_ms'if program=='framebuffer'else'cpu_frame_ms','gpu_commands_ms':'gpu_frame_ms'}
  for name,key in mapping.items():
   rows[name]={}
   for quantile in ['mean','p50','p95','p99']:
    values=[s[key][quantile]/(1e6 if program=='nightwatch'else 1)for s in samples if s.get(key) is not None and s[key].get('samples',0)>0]
    rows[name][quantile]=statistics.median(values)if values else None
  if program=='nightwatch':
   rows['cpu_prepare_plus_submit_mean_ms']=statistics.median((s['renderer_prepare']['mean']+s['submit']['mean'])/1e6 for s in samples)
   rows['draws_mean']=statistics.median(s['draws']['mean']for s in samples);rows['measured_frames']=samples[0]['window_samples'];rows['warmup_frames']=64
  else:
   rows['rust_allocations_per_frame']=statistics.median(s['rust_allocations']/s['frames']for s in samples)
   rows['rust_allocated_bytes_per_frame']=statistics.median(s['rust_allocated_bytes']/s['frames']for s in samples)
   rows['measured_frames']=samples[0]['frames'];rows['warmup_frames']=65 if program=='framebuffer'else 64
  return rows
 for program in ['nightwatch','quasar-composition','framebuffer']:
  for policy in (['poll']if program=='nightwatch'else['sync','poll']):
   key=program+'-'+policy;report['results'][key]={}
   for kind in ['baseline','native']:
    paths=sorted((out/'nightwatch').glob(kind+'-*-0.json'))if program=='nightwatch'else sorted(out.glob(f'{program}-{policy}-{kind}-*.json'))
    assert len(paths)==3,(paths,program,kind);report['results'][key][kind]=metrics(paths,program)
 if (root/'target/release/nightwatch').exists():
  report['versions']={kind:S.check_output([str(directory/'nightwatch'),'--version'],text=True).strip()for kind,directory in bins.items()}
 hashes={kind:[re.search(r'hash=([a-f0-9]+)',p.read_text()).group(1)for p in sorted((out/'nightwatch').glob(kind+'-*-0.log'))]for kind in bins};report['nightwatch_hashes']=hashes;assert len(set(hashes['baseline']+hashes['native']))==1,hashes
 (out/'summary.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2),flush=True)
finally:
 server.terminate();server.wait(timeout=10);log.close()
