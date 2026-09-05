const mineflayer = require('mineflayer');
const { Vec3 } = require('vec3');

// Usage: node dump_terrain.js <host> <port> <cx> <cz> [region] [username]
const [, , host, port, cx, cz, regionStr, username] = process.argv;
const region = parseInt(regionStr || '32');
const name = username || 'bot';
const center = new Vec3(parseInt(cx), 100, parseInt(cz));

const bot = mineflayer.createBot({ host, port: parseInt(port), username: name, version: '26.1' });

function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

bot.on('login', async () => {
    console.error(`[${name}] logged in, teleporting to ${center.x},${center.z}`);
    bot.chat(`/tp ${center.x} ${center.y} ${center.z}`);
    // Wait for the teleport + the chunk load around the target.
    await sleep(9000);

    const half = Math.floor(region / 2);
    const out = [];
    for (let dx = -half; dx < half; dx++) {
        const row = [];
        for (let dz = -half; dz < half; dz++) {
            const wx = center.x + dx;
            const wz = center.z + dz;
            let top = -1, nm = '?';
            for (let y = 319; y >= -64; y--) {
                const b = bot.blockAt(new Vec3(wx, y, wz));
                if (b && b.name !== 'air') { top = y; nm = b.name; break; }
            }
            row.push(`${top}:${nm}`);
        }
        out.push(row.join(' '));
    }
    console.log(out.join('\n'));
    bot.quit();
});

bot.on('error', e => console.error('bot error:', e.message));
bot.on('kicked', r => console.error('kicked:', r));
