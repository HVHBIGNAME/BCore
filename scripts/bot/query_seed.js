const mineflayer = require('mineflayer');
const [host = '127.0.0.1', port = '25571', username = 'bot'] = process.argv.slice(2);
const bot = mineflayer.createBot({ host, port: Number(port), username, version: '26.1' });
const timeout = setTimeout(() => finish(new Error('Seed query timed out')), 15000);
let finished = false;

function finish(error, seed) {
  if (finished) return;
  finished = true;
  clearTimeout(timeout);
  if (error) {
    console.error(error.message);
    process.exitCode = 1;
  } else {
    console.log(seed);
  }
  bot.quit();
}

bot.once('spawn', () => bot.chat('/seed'));
bot.on('message', message => {
  const match = message.toString().match(/Seed:\s*\[?(-?\d+)/i);
  if (match) finish(null, match[1]);
});
bot.on('error', error => finish(error));
bot.on('kicked', reason => finish(new Error(String(reason))));
