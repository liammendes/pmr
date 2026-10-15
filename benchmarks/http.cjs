// The same single-process HTTP workload is used for every manager and the baseline.
const http = require('node:http');
const server = http.createServer((req, res) => {
  res.setHeader('Content-Type', 'application/json');
  res.end(req.url === '/health' ? JSON.stringify({ pid: process.pid }) : '{"ok":true}\n');
});
server.listen(Number(process.env.PMR_BENCH_PORT), '127.0.0.1', () => {
  if (process.send) process.send('ready');
});
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
