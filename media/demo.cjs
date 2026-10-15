// Real supervised demo work. camera-sim generates synthetic frames, not camera footage.
const http = require('node:http');
const crypto = require('node:crypto');
const fs = require('node:fs');
const role = process.argv[2] || 'api';
let count = 0;
const metric = (name, value) => {
  if (process.send) process.send({type: 'axm:monitor', data: {[name]: {value}}});
};
console.log(`${role} ready | local showcase`);
if (role === 'api' || role === 'docs') {
  const port = role === 'api' ? 18321 : 18322;
  http.createServer((_req, res) => {
    count++;
    res.end('pmr demo is healthy\n');
    metric('HTTP responses', count);
    console.log(`GET /health 200 | response ${count}`);
  }).listen(port, '127.0.0.1');
  console.log(`Listening on 127.0.0.1:${port}`);
} else {
  setInterval(() => {
    const digest = crypto.createHash('sha256').update(Buffer.alloc(4096, count % 256)).digest('hex');
    count++;
    metric(role === 'camera-sim' ? 'Synthetic frames' : 'Jobs completed', count);
    console.log(`${role === 'camera-sim' ? 'Synthetic frame' : 'Job'} ${count} | checksum ${digest.slice(0,8)} | complete`);
    if (role === 'camera-sim' && count === 5 && !fs.existsSync('recovery.marker')) {
      fs.writeFileSync('recovery.marker', 'Intentional demo crash; next process stays alive.\n');
      console.error('Simulating a capture-process crash for the recovery demo');
      process.exit(1);
    }
  }, 2000);
}
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => process.exit(0));
