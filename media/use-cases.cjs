// Scripted agent events and real local background jobs for PMR's launch demo.
// No model API, external services, uploads or user files are involved.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const zlib = require('node:zlib');
const role = process.argv[2];
const agent = role?.startsWith('agent-');
const allowed = ['agent-planner', 'agent-builder', 'agent-reviewer', 'report-worker', 'archive-worker', 'checksum-worker'];
if (!allowed.includes(role)) throw new Error('Choose a supported demo role: ' + allowed.join(', '));
const output = path.resolve('showcase-output', role);
fs.mkdirSync(output, {recursive:true});
let count = 0;
const metric = (name,value) => process.send?.({type:'axm:monitor',data:{[name]:{value}}});
const prompts = ['Summarise the local fixture', 'Build a report from the fixture', 'Validate the report schema'];
const phases = {
  'agent-planner': ['input: '+prompts[0], 'read: 24 fixture records', 'plan: group, count, validate', 'output: 3 local steps queued'],
  'agent-builder': ['input: '+prompts[1], 'tool: parse fixture records', 'tool: write JSON report', 'output: report artifact saved'],
  'agent-reviewer': ['input: '+prompts[2], 'check: required keys present', 'check: record count = 24', 'output: local checks passed']
};
console.log(agent ? 'Scripted agent demo | real supervised process' : 'Local background-job demo | real file processing');
const run = () => {
  count++;
  const records = Array.from({length:24},(_,i)=>({id:i+1,value:(i+1)*count,group:i%3}));
  const data = Buffer.from(JSON.stringify(records));
  const digest = crypto.createHash('sha256').update(data).digest('hex');
  const report = {job:count,records:records.length,bytes:data.length,sha256:digest};
  const file = path.join(output,'latest.json');
  fs.writeFileSync(file,JSON.stringify(report,null,2)+'\n');
  if(agent){
    const phase=(count-1)%4;
    const task=String(Math.floor((count-1)/4)+1).padStart(3,'0');
    console.log('task-'+task+' | '+phases[role][phase]);
    metric('Demo events',count);
  } else {
    if(role==='archive-worker')fs.writeFileSync(path.join(output,'latest.json.gz'),zlib.gzipSync(data));
    if(role==='checksum-worker' && crypto.createHash('sha256').update(data).digest('hex')!==digest)throw new Error('Checksum mismatch');
    console.log('job-'+String(count).padStart(4,'0')+' | '+(role==='archive-worker'?'gzip archive':role==='checksum-worker'?'SHA-256 check':'JSON report')+' | 24 records | complete');
    metric('Jobs completed',count);
    metric('Records processed',count*24);
  }
};
run();
const timer=setInterval(run,agent?660:1300);
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>{clearInterval(timer);process.exit(0)});
