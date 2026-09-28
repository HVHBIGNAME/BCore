import java.nio.*;
import java.security.MessageDigest;
import java.lang.reflect.*;
import java.util.*;

/** Native piece postProcess against explicitly defined terrain and clipping. */
public class MineshaftBlockReference extends OreReference {
    static Object lookup, biome, blockingBiome, entityLevel, waterState, sand, shapeRandom;
    static Map<Pos,Object> writes, blockEntities;
    static List<Object> minecarts;
    static String scenario;
    static int[] shift = {0,0,0};
    static Object[] protectedStates;

    static Object initialBlock(Pos p) throws Exception {
        if (p.y() < -64 || p.y() >= 320) return air;
        p = new Pos(p.x()-shift[0],p.y()-shift[1],p.z()-shift[2]);
        if (p.y() >= (scenario.equals("structure_solid") ? 320 : 96)) return air;
        return switch (scenario) {
            case "liquid" -> waterState;
            case "sky" -> p.y() < 32 ? stone : air;
            case "pillars" -> p.y() <= 20 || p.y() >= 50 ? stone : air;
            case "hanging" -> p.y() >= 42 ? stone : air;
            case "gravity" -> p.y() >= 42 ? sand : air;
            case "long_down" -> p.y() <= 10 || p.y() == 95 ? stone : air;
            case "too_low" -> p.y() <= 9 || p.y() == 95 ? stone : air;
            case "long_up" -> p.y() >= 82 ? stone : air;
            case "too_high" -> p.y() >= 83 ? stone : air;
            case "lava_below" -> p.y() == 20 ? state("LAVA") : p.y() >= 42 ? stone : air;
            case "inside_water" -> p.x() == 5 && p.y() == 33 && p.z() == 7 ? waterState : stone;
            case "protected" -> p.x() == 5 && p.y() == 33 ? protectedStates[Math.floorMod(p.z(),protectedStates.length)] : stone;
            default -> stone;
        };
    }
    static Object block(Pos p) throws Exception { Object value = writes.get(p); return value == null ? initialBlock(p) : value; }
    static Object blockEntity(Pos p) throws Exception {
        Object state = block(p);
        if (!(boolean) call(state,"hasBlockEntity")) return null;
        if (!blockEntities.containsKey(p)) blockEntities.put(p,call(call(state,"getBlock"),"newBlockEntity",make("core.BlockPos",p.x(),p.y(),p.z()),state));
        return blockEntities.get(p);
    }
    static Object level() throws Exception {
        return Proxy.newProxyInstance(MineshaftBlockReference.class.getClassLoader(),new Class<?>[]{type("world.level.WorldGenLevel")},(p,m,a) -> switch(m.getName()) {
            case "getMinY" -> -64;
            case "getMaxY" -> 319;
            case "getHeight" -> {
                if (a == null || a.length == 0) yield 384;
                int x=(int)a[1], z=(int)a[2], top=-64;
                for (int y=319;y>=-64;y--) if ((boolean)call(block(new Pos(x,y,z)),"blocksMotion")) { top=y+1; break; }
                yield top;
            }
            case "getBlockState" -> block(Pos.from(a[0]));
            case "getFluidState" -> call(block(Pos.from(a[0])),"getFluidState");
            case "getBiome" -> scenario.equals("blocked") ? blockingBiome : biome;
            case "getBlockEntity" -> blockEntity(Pos.from(a[0]));
            case "getLevel" -> entityLevel;
            case "getRandom" -> shapeRandom;
            case "addFreshEntity" -> { minecarts.add(a[0]); yield true; }
            case "setBlock" -> {
                Pos pos=Pos.from(a[0]);
                if (pos.y() < -64 || pos.y() >= 320) yield false;
                if (call(block(pos),"getBlock") != call(a[1],"getBlock")) blockEntities.remove(pos);
                writes.put(pos,a[1]); blockEntity(pos); yield true;
            }
            case "getChunk" -> {
                Pos pos = a[0] instanceof Integer ? new Pos((int)a[0],0,(int)a[1]) : new Pos(Pos.from(a[0]).x()>>4,0,Pos.from(a[0]).z()>>4);
                yield chunk(pos.x(),pos.z());
            }
            case "scheduleTick" -> null;
            default -> throw new UnsupportedOperationException(m.toString());
        });
    }
    static Object pieceTag(String kind,String dir,int material) throws Exception {
        boolean alongZ=dir.equals("NORTH")||dir.equals("SOUTH");
        int orientation=(int)call(field("core.Direction",dir),"get2DDataValue");
        Map<String,Object> tag=new LinkedHashMap<>();
        tag.put("GD",1); tag.put("MST",material); tag.put("O",orientation);
        String id;
        if (kind.startsWith("corridor")) {
            id="mscorridor"; tag.put("BB",alongZ ? new int[]{4,32,4,6,34,23} : new int[]{4,32,4,23,34,6});
            tag.put("hr",kind.equals("corridor_rails")); tag.put("sc",kind.equals("corridor_spider")); tag.put("hps",false); tag.put("Num",4);
        } else if (kind.startsWith("crossing")) {
            id="mscrossing"; tag.put("BB",new int[]{4,32,4,8,kind.equals("crossing_two")?38:34,8});
            tag.put("D",orientation); tag.put("O",-1); tag.put("tf",kind.equals("crossing_two"));
        } else if (kind.equals("stairs")) {
            id="msstairs"; tag.put("BB",alongZ ? new int[]{4,27,4,6,34,12} : new int[]{4,27,4,12,34,6});
        } else {
            id="msroom"; tag.put("O",-1); tag.put("BB",new int[]{4,32,4,13,39,14});
            tag.put("Entrances",List.of(new int[]{4,33,5,5,35,7},new int[]{8,33,4,10,35,5}));
        }
        tag.put("id","minecraft:"+id);
        Object gson=Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        Object json=call(Class.forName("com.google.gson.JsonParser"),"parseString",call(gson,"toJson",tag));
        Object ops=Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        Object nbt=call(ops,"convertTo",field("nbt.NbtOps","INSTANCE"),json);
        // BoundingBox codecs require NBT int arrays rather than homogeneous lists.
        call(nbt,"putIntArray","BB",tag.get("BB"));
        return nbt;
    }
    static Map<String,Object> capture(String kind,String dir,int material,long seed,String terrain,String clipping,int[] offset) throws Exception {
        shift=offset;
        scenario=terrain; writes=new HashMap<>(); blockEntities=new HashMap<>(); minecarts=new ArrayList<>();
        chunks=new HashMap<>(); OreReference.terrain="air"; base=stone; baseFactory=factory(stone); surface=96;
        Object tag=pieceTag(kind,dir,material);
        String suffix=kind.startsWith("corridor")?"Corridor":kind.startsWith("crossing")?"Crossing":kind.equals("stairs")?"Stairs":"Room";
        Object piece=make("world.level.levelgen.structure.structures.MineshaftPieces$MineShaft"+suffix,tag);
        call(piece,"move",offset[0],offset[1],offset[2]);
        tag=call(piece,"createTag",(Object)null);
        Object random=make("world.level.levelgen.WorldgenRandom",make("world.level.levelgen.XoroshiroRandomSource",seed));
        List<int[]> clips=new ArrayList<>();
        if(clipping.equals("full")) clips.add(new int[]{-64+offset[0],-64,-64+offset[2],64+offset[0],319,64+offset[2]});
        else {
            Object bounds=call(piece,"getBoundingBox");
            int x0=(int)call(bounds,"minX")>>4, z0=(int)call(bounds,"minZ")>>4;
            int x1=(int)call(bounds,"maxX")>>4, z1=(int)call(bounds,"maxZ")>>4;
            if(clipping.equals("first")) { x1=x0; z1=z0; }
            for(int x=x0;x<=x1;x++) for(int z=z0;z<=z1;z++) clips.add(new int[]{x*16,-64,z*16,x*16+15,319,z*16+15});
            if(clipping.equals("reverse")) Collections.reverse(clips);
        }
        List<Object> pieceStates=new ArrayList<>();
        Object jsonOps=Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        for(int[] clip:clips) {
            Object box=make("world.level.levelgen.structure.BoundingBox",clip[0],clip[1],clip[2],clip[3],clip[4],clip[5]);
            call(piece,"postProcess",level(),null,null,random,box,make("world.level.ChunkPos",clip[0]>>4,clip[2]>>4),make("core.BlockPos",0,32,0));
            pieceStates.add(call(field("nbt.NbtOps","INSTANCE"),"convertTo",jsonOps,call(piece,"createTag",(Object)null)));
        }
        MessageDigest digest=MessageDigest.getInstance("MD5"); int changed=0;
        List<Pos> positions=new ArrayList<>(writes.keySet()); positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        for(Pos p:positions) {
            Object state=block(p); if(state==initialBlock(p)) continue;
            int id=(int)call(type("world.level.block.Block"),"getId",state);
            digest.update(ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN).putInt(p.x()).putInt(p.y()).putInt(p.z()).putInt(id).array()); changed++;
        }
        List<Object> entities=new ArrayList<>();
        for(Object cart:minecarts) {
            Object problems=make("util.ProblemReporter$Collector");
            Object output=call(type("world.level.storage.TagValueOutput"),"createWithContext",problems,lookup);
            if(!(boolean)call(cart,"save",output)) throw new IllegalStateException("native minecart failed to save");
            if(!(boolean)call(problems,"isEmpty")) throw new IllegalStateException((String)call(problems,"getReport"));
            Object saved=call(output,"buildResult");
            // UUID is generated by the entity's independent, unseeded RNG.
            // It must not be conflated with the deterministic placement stream.
            call(saved,"remove","UUID");
            entities.add(Map.of("pos",List.of(call(cart,"getX"),call(cart,"getY"),call(cart,"getZ")),
                "loot_seed",call(cart,"getContainerLootTableSeed"),"loot_table",call(call(cart,"getContainerLootTable"),"identifier").toString(),
                "nbt",call(field("nbt.NbtOps","INSTANCE"),"convertTo",jsonOps,saved),"snbt",saved.toString(),
                "type",call(field("core.registries.BuiltInRegistries","ENTITY_TYPE"),"getId",call(cart,"getType"))));
        }
        List<Object> bes=new ArrayList<>();
        positions=new ArrayList<>(blockEntities.keySet()); positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        for(Pos p:positions) bes.add(call(field("nbt.NbtOps","INSTANCE"),"convertTo",jsonOps,call(blockEntities.get(p),"saveWithFullMetadata",lookup)));
        List<Pos> marked=new ArrayList<>();
        Method unpack=type("world.level.chunk.ProtoChunk").getMethod("unpackOffsetCoordinates",short.class,int.class,type("world.level.ChunkPos"));
        for(Object chunk:chunks.values()) {
            Object[] sections=(Object[])call(chunk,"getPostProcessing");
            for(int s=0;s<sections.length;s++) if(sections[s]!=null)
                for(Object packed:(Iterable<?>)sections[s]) marked.add(Pos.from(unpack.invoke(null,packed,s-4,call(chunk,"getPos"))));
        }
        marked.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        Map<String,Object> result=new LinkedHashMap<>();
        result.put("piece",call(field("nbt.NbtOps","INSTANCE"),"convertTo",jsonOps,tag)); result.put("seed",seed); result.put("terrain",terrain); result.put("clips",clips); result.put("shift",shift);
        result.put("changed_blocks",changed); result.put("writes_md5",HexFormat.of().formatHex(digest.digest())); result.put("next_i64",call(random,"nextLong"));
        result.put("entities",entities); result.put("block_entities",bes); result.put("piece_states",pieceStates);
        result.put("postprocessing",marked.stream().map(p->List.of(p.x(),p.y(),p.z())).toList());
        return result;
    }
    static int stateId(Object state) throws Exception { return (int)call(type("world.level.block.Block"),"getId",state); }

    static Map<String,Object> predicates() throws Exception {
        scenario="stone"; shift=new int[]{0,0,0}; writes=new HashMap<>();
        Object world=level(), pos=make("core.BlockPos",0,32,0);
        Object piece=make("world.level.levelgen.structure.structures.MineshaftPieces$MineShaftCorridor",pieceTag("corridor_plain","SOUTH",0));
        List<Object> ranges=new ArrayList<>(); int start=0, last=-1, id=0;
        Set<Object> oak=new HashSet<>(), dark=new HashSet<>();
        for(String name:List.of("OAK_PLANKS","OAK_LOG","OAK_FENCE","IRON_CHAIN")) oak.add(field("world.level.block.Blocks",name));
        for(String name:List.of("DARK_OAK_PLANKS","DARK_OAK_LOG","DARK_OAK_FENCE","IRON_CHAIN")) dark.add(field("world.level.block.Blocks",name));
        Object[] directions=(Object[])call(type("core.Direction"),"values");
        Object fence=field("world.level.block.Blocks","OAK_FENCE");
        for(Object state:(Iterable<?>)field("world.level.block.Block","BLOCK_STATE_REGISTRY")) {
            writes.put(new Pos(0,32,0),state);
            Object block=call(state,"getBlock");
            int flags=((boolean)call(state,"liquid")?1:0) | ((boolean)call(piece,"isReplaceableByStructures",state)?2:0)
                | ((boolean)call(state,"isSolidRender")?4:0) | ((boolean)call(piece,"canHangChainBelow",world,pos,state)?8:0)
                | (oak.contains(block)?1024:0) | (dark.contains(block)?2048:0) | (block==field("world.level.block.Blocks","LAVA")?4096:0)
                | ((boolean)call(state,"blocksMotion")?8192:0);
            for(int d=0;d<directions.length;d++) if((boolean)call(state,"isFaceSturdy",world,pos,directions[d])) flags |= 16<<d;
            for(int d=2;d<directions.length;d++)
                if((boolean)call(fence,"connectsTo",state,(flags&(16<<d))!=0,directions[d])) flags |= 16384<<(d-2);
            if(flags!=last) { if(id>0) ranges.add(List.of(start,id,last)); start=id;last=flags; }
            id++;
        }
        ranges.add(List.of(start,id,last));
        return Map.of("ranges",ranges,"state_count",id);
    }

    static Map<String,Object> materials() throws Exception {
        Map<String,Object> result=new TreeMap<>();
        for(String name:List.of("OAK_PLANKS","OAK_LOG","OAK_FENCE","DARK_OAK_PLANKS","DARK_OAK_LOG","DARK_OAK_FENCE","IRON_CHAIN","CAVE_AIR","COBWEB","RAIL","SPAWNER","SAND")) result.put(name,stateId(state(name)));
        for(int material=0;material<2;material++) {
            Map<String,Object> variants=new LinkedHashMap<>();
            Object fence=state(material==0?"OAK_FENCE":"DARK_OAK_FENCE");
            variants.put("fence_west",call(fence,"setValue",field("world.level.block.FenceBlock","WEST"),true));
            variants.put("fence_east",call(fence,"setValue",field("world.level.block.FenceBlock","EAST"),true));
            for(String dir:List.of("NORTH","SOUTH")) variants.put("torch_"+dir.toLowerCase(Locale.ROOT),call(state("WALL_TORCH"),"setValue",field("world.level.block.WallTorchBlock","FACING"),field("core.Direction",dir)));
            for(String shape:List.of("NORTH_SOUTH","EAST_WEST")) variants.put("rail_"+shape.toLowerCase(Locale.ROOT),call(state("RAIL"),"setValue",field("world.level.block.RailBlock","SHAPE"),field("world.level.block.state.properties.RailShape",shape)));
            Map<String,Object> oriented=new TreeMap<>();
            for(var entry:variants.entrySet()) {
                List<Integer> ids=new ArrayList<>();
                for(String direction:List.of("SOUTH","WEST","NORTH","EAST")) {
                    Object piece=make("world.level.levelgen.structure.structures.MineshaftPieces$MineShaftCorridor",pieceTag("corridor_plain",direction,material));
                    Object state=call(call(entry.getValue(),"mirror",call(piece,"getMirror")),"rotate",call(piece,"getRotation"));
                    ids.add(stateId(state));
                }
                oriented.put(entry.getKey(),ids);
            }
            result.put(material==0?"normal":"mesa",oriented);
        }
        return result;
    }
    static List<Object> shapeChecks() throws Exception {
        List<Object> result=new ArrayList<>();
        Object pos=make("core.BlockPos",0,32,0), world=level();
        Object[] directions=(Object[])call(type("core.Direction"),"values");
        scenario="sky"; shift=new int[]{0,0,0};
        for(String name:List.of("OAK_FENCE","DARK_OAK_FENCE","WALL_TORCH"))
            for(String neighbor:List.of("AIR","STONE","OAK_FENCE","DARK_OAK_FENCE","NETHER_BRICK_FENCE","OAK_FENCE_GATE","OAK_LEAVES","GLASS","BARRIER","PUMPKIN","SHULKER_BOX","STONE_SLAB"))
                for(int side=2;side<6;side++) {
                    writes=new HashMap<>();
                    Object current=state(name), adjacent=state(neighbor);
                    writes.put(new Pos(0,32,0),current);
                    List<Integer> neighbours=new ArrayList<>();
                    for(int d=0;d<directions.length;d++) {
                        Object value=d==side?adjacent:air;
                        writes.put(Pos.from(call(pos,"relative",directions[d])),value); neighbours.add(stateId(value));
                    }
                    Object updated=call(type("world.level.block.Block"),"updateFromNeighbourShapes",current,world,pos);
                    result.add(Map.of("state",stateId(current),"neighbors",neighbours,"updated",stateId(updated)));
                }
        return result;
    }
    public static void main(String[] args) throws Exception {
        bootstrapOre(); lookup=call(type("data.registries.VanillaRegistries"),"createLookup");
        Object biomeRegistry=call(lookup,"lookupOrThrow",field("core.registries.Registries","BIOME"));
        biome=call(biomeRegistry,"getOrThrow",field("world.level.biome.Biomes","PLAINS"));
        call(biome,"bindTags",Set.of());
        blockingBiome=call(biomeRegistry,"getOrThrow",field("world.level.biome.Biomes","DEEP_DARK"));
        call(blockingBiome,"bindTags",Set.of(field("tags.BiomeTags","MINESHAFT_BLOCKING")));
        Map<Object,Set<Object>> tags=new HashMap<>();
        for(String name:List.of("unstable_bottom_center","fences","wooden_fences","shulker_boxes")) {
            Object tag=field("tags.BlockTags",name.toUpperCase(Locale.ROOT));
            for(Object block:tagBlocks(name)) tags.computeIfAbsent(block,k->new HashSet<>()).add(tag);
        }
        for (Object block : (Iterable<?>)field("core.registries.BuiltInRegistries","BLOCK"))
            call(call(block,"builtInRegistryHolder"),"bindTags",tags.getOrDefault(block,Set.of()));
        entityLevel=NativeEntityLevel.create(); waterState=state("WATER"); sand=state("SAND");
        shapeRandom=make("world.level.levelgen.LegacyRandomSource",0L);
        protectedStates=new Object[]{state("OAK_PLANKS"),state("OAK_LOG"),state("OAK_FENCE"),state("IRON_CHAIN"),state("DARK_OAK_PLANKS"),state("DARK_OAK_LOG"),state("DARK_OAK_FENCE"),stone};
        List<Object> samples=new ArrayList<>();
        List<String> directions=List.of("NORTH","SOUTH","WEST","EAST");
        List<String> kinds=List.of("room","stairs","crossing","crossing_two","corridor_rails","corridor_spider","corridor_plain");
        int i=0;
        for(String kind:kinds) {
            for(String terrain:List.of("stone","pillars","hanging","sky","liquid","blocked","protected","gravity","long_down","long_up","too_low","too_high","lava_below","inside_water"))
                for(int material=0;material<2;material++)
                    for(long seed:new long[]{0,42}) samples.add(capture(kind,directions.get(i++%4),material,seed,terrain,"full",new int[]{0,0,0}));
            for(String dir:directions) for(String mode:List.of("first","forward","reverse"))
                samples.add(capture(kind,dir,0,17,"stone",mode,new int[]{-8,0,-8}));
            for(int y:new int[]{-94,283}) samples.add(capture(kind,"SOUTH",1,-1,"hanging","full",new int[]{-16,y,16}));
        }
        for (long seed=0;seed<128;seed++) samples.add(capture("corridor_plain",directions.get((int)seed%4),(int)seed%2,seed,"stone","full",new int[]{0,0,0}));
        Object gson=Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        Map<String,Object> result=new LinkedHashMap<>(predicates());
        result.put("materials",materials()); result.put("samples",samples); result.put("shape_checks",shapeChecks());
        System.out.println("MINESHAFT_BLOCKS_REFERENCE="+call(gson,"toJson",result));
    }
}
