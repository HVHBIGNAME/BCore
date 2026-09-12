const mineflayer = require('mineflayer');
const { Vec3 } = require('vec3');

// Usage: node dump_terrain.js <host> <port> <cx> <cz> [region] [username] [--ground] [--full]
const argv = process.argv.slice(2);
const full = argv.includes('--full');
// `--ground` reads the terrain height (world_surface) instead of the top
// non-air block, so the comparison isolates density from tree foliage.
const ground = argv.includes('--ground');
const [host, port, cx, cz, regionStr, username] = argv.filter((a) => a !== '--ground');
const region = parseInt(regionStr || '32');
const name = username || 'bot';
const center = new Vec3(parseInt(cx), 100, parseInt(cz));

const bot = mineflayer.createBot({ host, port: parseInt(port), username: name, version: '26.1' });
let biomeNames = [];
bot._client.on('registry_data', packet => {
    if (packet.id === 'minecraft:worldgen/biome') biomeNames = packet.entries.map(entry => entry.key);
});

function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

bot.on('login', async () => {
    console.error(`[${name}] logged in, teleporting to ${center.x},${center.z}`);
    // Spectator first: a survival/creative bot teleported to y=100 lands INSIDE
    // the terrain at many coordinates, suffocates in a wall, dies and respawns
    // at the world spawn — the dump then silently measures the wrong place.
    bot.chat('/gamemode spectator');
    await sleep(1000);
    // Teleport high above the surface so the column is loaded for reading.
    bot.chat(`/tp ${center.x} ${Math.max(center.y, 250)} ${center.z}`);
    // Wait for the teleport + the chunk load around the target. Far chunks
    // (and ocean aquifers) can take the vanilla server well over 9s to
    // generate and ship, so give it a generous window.
    await sleep(25000);

    const dxp = bot.entity.position.x - center.x;
    const dzp = bot.entity.position.z - center.z;
    console.error(`[${name}] position after teleport: ${bot.entity.position.x.toFixed(2)},${bot.entity.position.y.toFixed(2)},${bot.entity.position.z.toFixed(2)}`);
    if (Math.hypot(dxp, dzp) > 8) {
        console.error(`FATAL: teleport verification failed: expected near ${center.x},${center.z}, got ${bot.entity.position.x},${bot.entity.position.z}`);
        process.exitCode = 2;
        bot.quit();
        return;
    }
    const half = Math.floor(region / 2);
    const out = [];
    for (let dz = -half; dz < half; dz++) {
        const row = [];
        for (let dx = -half; dx < half; dx++) {
            const wx = center.x + dx;
            const wz = center.z + dz;
            let top = -1, nm = '?';
            for (let y = 319; y >= -64; y--) {
                const b = bot.blockAt(new Vec3(wx, y, wz));
                if (b && b.name !== 'air') {
                    // Skip no-collision blocks (leaves, grass, plants) when the
                    // caller wants the terrain height (world_surface) rather
                    // than the top non-air block.
                    // `--ground` mirrors vanilla's MOTION_BLOCKING_NO_LEAVES:
                    // non-colliding blocks AND leaves are skipped. The leaf
                    // check must be name-based because minecraft-data reports
                    // leaves with boundingBox 'block'.
                    if (
                        ground &&
                        (b.boundingBox === 'empty' ||
                            b.name.endsWith('_leaves') ||
                            b.name.endsWith('_log'))
                    )
                        continue;
                    top = y; nm = b.name; break;
                }
            }
            row.push(`${top}:${nm}`);
        }
        out.push(row.join(' '));
    }
    if (full) {
        const blocks = [];
        const columns = [];
        const biomes = [];
        // Y range is caller-controlled: a hardcoded 40..120 clipped every tree
        // whose ground sits above ~120 (e.g. ground 126 at (0,0)), which made
        // vanilla/Bcore tree-origin comparisons read 0 on BOTH sides.
        const _numArg = (prefix, dflt) => {
            const hit = argv.find(a => a.startsWith(prefix));
            return hit ? parseInt(hit.split('=')[1], 10) : dflt;
        };
        const ymin = _numArg('--ymin=', 40), ymax = _numArg('--ymax=', 120);
        for (let dx = -half; dx < half; dx++) {
            for (let dz = -half; dz < half; dz++) {
                const wx = center.x + dx, wz = center.z + dz;
                let top = null, terrain = null;
                for (let y = ymin; y <= ymax; y++) {
                    const b = bot.blockAt(new Vec3(wx, y, wz));
                    if (!b) throw new Error(`Unloaded block at ${wx},${y},${wz}`);
                    blocks.push([wx, y, wz, b.name, b.stateId]);
                    if (!['air', 'cave_air', 'void_air'].includes(b.name)) top = y;
                    if (b.boundingBox !== 'empty' && !b.name.endsWith('_leaves') && !b.name.endsWith('_log')) terrain = y;
                    if (wx % 4 === 0 && y % 4 === 0 && wz % 4 === 0) {
                        const biomeName = biomeNames[b.biome.id];
                        if (!biomeName) throw new Error(`Unknown biome ID ${b.biome.id}`);
                        biomes.push([wx, y, wz, biomeName]);
                    }
                }
                columns.push([wx, wz, top, terrain]);
            }
        }
        console.log(JSON.stringify({center: [center.x, center.z], position: [bot.entity.position.x, bot.entity.position.y, bot.entity.position.z], ymin, ymax, blocks, columns, biomes}));
    } else {
        console.log(out.join('\n'));
    }
    bot.quit();
    // Explicit exit: the socket close can keep the event loop alive, which
    // hangs the harness. Give it a moment to flush, then exit regardless.
    setTimeout(() => process.exit(0), 200);
});

bot.on('error', e => {
    console.error('bot error:', e.message);
    process.exitCode = 1;
});
bot.on('kicked', r => {
    console.error('kicked:', r);
    process.exitCode = 1;
});
