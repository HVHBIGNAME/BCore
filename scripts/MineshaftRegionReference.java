import java.lang.reflect.*;
import java.nio.*;
import java.security.MessageDigest;
import java.util.*;

/** Native StructureStart.placeInChunk across overlapping starts and chunk edges.
 * Terrain is solid stone, with no other feature stages or gameplay ticks.
 */
public class MineshaftRegionReference extends MineshaftBlockReference {
    static Object jsonOps, structures;
    static Object jsonTag(Object tag) throws Exception { return call(field("nbt.NbtOps","INSTANCE"),"convertTo",jsonOps,tag); }
    static Object regionWorld() throws Exception {
        Object base=level(); InvocationHandler handler=Proxy.getInvocationHandler(base);
        return Proxy.newProxyInstance(MineshaftRegionReference.class.getClassLoader(),new Class<?>[]{type("world.level.WorldGenLevel")},(p,m,a)->switch(m.getName()) {
            case "getHeight" -> a==null || a.length==0 ? 384 : 320;
            case "getMinSectionY" -> -4;
            case "registryAccess" -> lookup;
            case "setBlock" -> {
                if(Pos.from(a[0]).y()>=319) throw new IllegalStateException("fixture's unchanged top layer was overwritten");
                yield handler.invoke(base,m,a);
            }
            default -> handler.invoke(base,m,a);
        });
    }
    static Map<String,Object> captureRegion(long seed,boolean mixed) throws Exception {
        writes=new HashMap<>(); blockEntities=new HashMap<>(); minecarts=new ArrayList<>(); chunks=new HashMap<>();
        scenario="structure_solid"; shift=new int[]{0,0,0}; terrain="air"; surface=320; baseFactory=airFactory;
        Map<Pos,Object> starts=new TreeMap<>(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::z));
        Set<Pos> targets=new TreeSet<>(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::z));
        List<Object> definitions=new ArrayList<>();
        Object manager=make("world.level.StructureManager",null,make("world.level.levelgen.WorldOptions",seed,true,false),null);
        MineshaftStartReference.configure(seed,"plains");
        Object placement=call(call(MineshaftStartReference.structureSet,"value"),"placement");
        for(int x=-20;x<=20;x++) for(int z=-20;z<=20;z++) {
            if(!(boolean)call(placement,"isStructureChunk",MineshaftStartReference.structureState,x,z)) continue;
            MineshaftStartReference.configure(seed,mixed && (x&1)!=0 ? "badlands" : "plains");
            Object chunk=chunk(x,z);
            call(MineshaftStartReference.generator,"createStructures",lookup,MineshaftStartReference.structureState,manager,chunk,null,field("world.level.Level","OVERWORLD"));
            for(var entry:((Map<?,?>)call(chunk,"getAllStarts")).entrySet()) {
                Object start=entry.getValue(); starts.put(new Pos(x,0,z),start);
                List<Object> pieces=new ArrayList<>();
                for(Object piece:(List<?>)call(start,"getPieces")) pieces.add(jsonTag(call(piece,"createTag",(Object)null)));
                definitions.add(Map.of("source",List.of(x,z),"id",call(structures,"getKey",entry.getKey()).toString(),"pieces",pieces));
                Object b=call(start,"getBoundingBox");
                for(int cx=(int)call(b,"minX")>>4;cx<=(int)call(b,"maxX")>>4;cx++)
                    for(int cz=(int)call(b,"minZ")>>4;cz<=(int)call(b,"maxZ")>>4;cz++) targets.add(new Pos(cx,0,cz));
            }
        }
        Object world=regionWorld();
        manager=make("world.level.StructureManager",world,make("world.level.levelgen.WorldOptions",seed,true,false),null);
        List<Object> resultChunks=new ArrayList<>();
        for(Pos target:targets) {
            Object chunk=chunk(target.x(),target.z());
            // Populate the same native LongOpenHashSet used by createReferences,
            // using the native bounding-box intersection and source scan order.
            for(var entry:starts.entrySet()) {
                Pos source=entry.getKey(); Object start=entry.getValue();
                if(Math.abs(source.x()-target.x())>8 || Math.abs(source.z()-target.z())>8) continue;
                if((boolean)call(call(start,"getBoundingBox"),"intersects",target.x()*16,target.z()*16,target.x()*16+15,target.z()*16+15))
                    call(chunk,"addReferenceForStructure",call(start,"getStructure"),call(type("world.level.ChunkPos"),"pack",source.x(),source.z()));
            }
            Object random=make("world.level.levelgen.WorldgenRandom",make("world.level.levelgen.XoroshiroRandomSource",seed));
            long decoration=(long)call(random,"setDecorationSeed",seed,target.x()*16,target.z()*16);
            Object section=call(type("core.SectionPos"),"bottomOf",chunk);
            Object clip=call(type("world.level.chunk.ChunkGenerator"),"getWritableArea",chunk);
            for(String name:List.of("mineshaft","mineshaft_mesa")) {
                Object structure=call(call(structures,"getOrThrow",MineshaftStartReference.key("STRUCTURE",name)),"value");
                call(random,"setFeatureSeed",decoration,name.equals("mineshaft")?1:2,3);
                for(Object start:(List<?>)call(manager,"startsForStructure",section,structure))
                    call(start,"placeInChunk",world,manager,MineshaftStartReference.generator,random,clip,make("world.level.ChunkPos",target.x(),target.z()));
            }
            Map<String,Object> record=new LinkedHashMap<>(); record.put("chunk",List.of(target.x(),target.z()));
            Map<String,Object> references=new TreeMap<>();
            for(var entry:((Map<?,?>)call(chunk,"getAllReferences")).entrySet()) {
                List<Object> values=new ArrayList<>();
                for(Object value:(Iterable<?>)entry.getValue()) values.add(value);
                references.put(call(structures,"getKey",entry.getKey()).toString(),values);
            }
            record.put("references",references); resultChunks.add(record);
        }
        // All cross-chunk calls have completed before taking the snapshots.
        for(Object item:resultChunks) {
            @SuppressWarnings("unchecked") Map<String,Object> record=(Map<String,Object>)item;
            List<?> target=(List<?>)record.get("chunk"); int cx=(int)target.get(0), cz=(int)target.get(1);
            List<Pos> positions=writes.keySet().stream().filter(p->p.x()>>4==cx && p.z()>>4==cz).sorted(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z)).toList();
            MessageDigest digest=MessageDigest.getInstance("MD5"); int count=0;
            for(Pos p:positions) if(block(p)!=initialBlock(p)) {
                digest.update(ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN).putInt(p.x()).putInt(p.y()).putInt(p.z()).putInt(stateId(block(p))).array()); count++;
            }
            record.put("changed_blocks",count);record.put("writes_md5",HexFormat.of().formatHex(digest.digest()));
            List<Object> bes=new ArrayList<>();
            for(Pos p:blockEntities.keySet().stream().filter(p->p.x()>>4==cx && p.z()>>4==cz).sorted(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z)).toList()) bes.add(jsonTag(call(blockEntities.get(p),"saveWithFullMetadata",lookup)));
            List<Object> carts=new ArrayList<>();
            for(Object cart:minecarts) if(((int)Math.floor((double)call(cart,"getX"))>>4)==cx && ((int)Math.floor((double)call(cart,"getZ"))>>4)==cz)
                carts.add(Map.of("pos",List.of(call(cart,"getX"),call(cart,"getY"),call(cart,"getZ")),"loot_seed",call(cart,"getContainerLootTableSeed")));
            record.put("block_entities",bes);record.put("minecarts",carts);
        }
        return Map.of("seed",seed,"mixed",mixed,"starts",definitions,"chunks",resultChunks);
    }
    public static void main(String[] args) throws Exception {
        bootstrapOre();lookup=NativeWorldgenRegistries.load();
        MineshaftStartReference.registries=lookup;
        MineshaftStartReference.settingsHolder=call(call(lookup,"lookupOrThrow",field("core.registries.Registries","NOISE_SETTINGS")),"getOrThrow",field("world.level.levelgen.NoiseGeneratorSettings","OVERWORLD"));
        MineshaftStartReference.settings=call(MineshaftStartReference.settingsHolder,"value");
        MineshaftStartReference.noises=call(lookup,"lookupOrThrow",field("core.registries.Registries","NOISE"));
        MineshaftStartReference.structureSet=call(call(lookup,"lookupOrThrow",field("core.registries.Registries","STRUCTURE_SET")),"getOrThrow",MineshaftStartReference.key("STRUCTURE_SET","mineshafts"));
        structures=call(lookup,"lookupOrThrow",field("core.registries.Registries","STRUCTURE"));
        biome=call(call(lookup,"lookupOrThrow",field("core.registries.Registries","BIOME")),"getOrThrow",MineshaftStartReference.key("BIOME","plains"));
        entityLevel=NativeEntityLevel.create(); jsonOps=Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        List<Object> samples=List.of(captureRegion(0,false),captureRegion(846692123413862008L,true));
        Object gson=Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("MINESHAFT_REGION_REFERENCE="+call(gson,"toJson",Map.of("samples",samples)));
    }
}
