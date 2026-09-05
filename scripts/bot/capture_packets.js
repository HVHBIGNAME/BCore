const fs = require('fs');
const path = require('path');
const mineflayer = require('mineflayer');

const [, , host, portArg, outArg, usernameArg] = process.argv;
if (!host || !portArg || !outArg) { console.error('usage: node capture_packets.js HOST PORT OUT.json [username]'); process.exit(2); }
const port = Number(portArg), out = path.resolve(outArg), username = usernameArg || 'capturebot';
const packets = [];
const started = Date.now();
function safe(v, depth=0) {
  if (depth > 5) return '[depth]';
  if (Buffer.isBuffer(v)) return {type:'Buffer', length:v.length};
  if (typeof v === 'bigint') return v.toString();
  if (v && typeof v === 'object') {
    if (Array.isArray(v)) return v.slice(0,200).map(x=>safe(x,depth+1));
    const o={}; for (const [k,x] of Object.entries(v)) o[k]=safe(x,depth+1); return o;
  }
  return v;
}
const bot = mineflayer.createBot({host, port, username, version:'26.1', checkTimeoutInterval: 30000});
const client = bot._client;
client.on('packet', (data, meta) => {
  if (Date.now()-started <= 3500) packets.push({seq:packets.length, t:Date.now()-started, state:meta.state, name:meta.name, data:safe(data)});
});
function finish(reason) {
  fs.mkdirSync(path.dirname(out), {recursive:true});
  fs.writeFileSync(out, JSON.stringify({host,port,username,durationMs:Date.now()-started,reason,packets},null,2));
  console.error(`wrote ${out}: ${packets.length} packets`);
  try { bot.quit(); } catch {}
  setTimeout(()=>process.exit(0),200);
}
bot.once('login', ()=>console.error('login event'));
bot.once('spawn', ()=>console.error('spawn event'));
bot.on('error', e=>console.error('error:',e.message));
bot.on('kicked', r=>console.error('kicked:',JSON.stringify(r)));
setTimeout(()=>finish('timeout'), 15000);
bot.on('end', ()=>finish('end'));
