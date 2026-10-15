const http = require('node:http');
const port = Number(process.env.PORT || 3000);
const server = http.createServer((_req, res) => {
  res.setHeader('Content-Type', 'application/json');
  res.end(JSON.stringify({ message: 'Hello from pmr', pid: process.pid }) + '\n');
});
server.listen(port, '127.0.0.1', () => {
  console.log(`API listening on http://127.0.0.1:${port}`);
  if (process.send) process.send('ready');
});
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
