import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;
import java.util.stream.Stream;

/** Native createStructures: frequency, weighted selection, biome/height admission. */
public class MineshaftStartReference extends OreReference {
    static Object registries, settingsHolder, settings, noises, structureSet;
    static Object source, generator, randomState, structureState;

    static Object key(String registry,String name) throws Exception {
        return call(type("resources.ResourceKey"),"create",field("core.registries.Registries",registry),call(type("resources.Identifier"),"withDefaultNamespace",name));
    }
    static String holderName(Object holder) throws Exception { return call(call(holder,"key"),"identifier").toString(); }
    static List<String> names(Object holderSet) throws Exception {
        List<String> result=new ArrayList<>();
        for(Object holder:(Iterable<?>)holderSet) result.add(holderName(holder));
        Collections.sort(result); return result;
    }
    static String canonical(Object json) throws Exception {
        if((boolean)call(json,"isJsonObject")) {
            Map<String,Object> values=new TreeMap<>();
            for(Object item:(Set<?>)call(call(json,"getAsJsonObject"),"entrySet")) {
                Map.Entry<?,?> entry=(Map.Entry<?,?>)item; values.put((String)entry.getKey(),entry.getValue());
            }
            List<String> fields=new ArrayList<>();
            for(var entry:values.entrySet()) fields.add("\""+entry.getKey()+"\":"+canonical(entry.getValue()));
            return "{"+String.join(",",fields)+"}";
        }
        if((boolean)call(json,"isJsonArray")) {
            List<String> values=new ArrayList<>();
            for(Object value:(Iterable<?>)call(json,"getAsJsonArray")) values.add(canonical(value));
            return "["+String.join(",",values)+"]";
        }
        return json.toString();
    }
    static List<Object> referenceOrders() throws Exception {
        List<Object> result=new ArrayList<>();
        for(int count:new int[]{1,2,16,24,25,48,100}) {
            Object set=Class.forName("it.unimi.dsi.fastutil.longs.LongOpenHashSet").getConstructor().newInstance();
            List<Object> inserted=new ArrayList<>(), iterated=new ArrayList<>();
            for(int i=0;i<count;i++) {
                int x=i==0?0:(i%2==0?-i:i), z=i%7-3;
                if(i==0) z=0;
                inserted.add(List.of(x,z)); call(set,"add",call(type("world.level.ChunkPos"),"pack",x,z));
            }
            for(Object packed:(Iterable<?>)set) {
                Object pos=call(type("world.level.ChunkPos"),"unpack",packed);
                iterated.add(List.of(call(pos,"x"),call(pos,"z")));
            }
            result.add(Map.of("inserted",inserted,"iterated",iterated));
        }
        return result;
    }
    static void configure(long seed,String biome) throws Exception {
        Object biomeRegistry=call(registries,"lookupOrThrow",field("core.registries.Registries","BIOME"));
        if(biome.equals("overworld")) {
            Object parameters=make("world.level.biome.MultiNoiseBiomeSourceParameterList",field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset","OVERWORLD"),biomeRegistry);
            source=call(type("world.level.biome.MultiNoiseBiomeSource"),"createFromPreset",call(type("core.Holder"),"direct",parameters));
        } else source=make("world.level.biome.FixedBiomeSource",call(biomeRegistry,"getOrThrow",key("BIOME",biome)));
        generator=make("world.level.levelgen.NoiseBasedChunkGenerator",source,settingsHolder);
        randomState=call(type("world.level.levelgen.RandomState"),"create",settings,noises,seed);
        structureState=call(type("world.level.chunk.ChunkGeneratorStructureState"),"createForFlat",randomState,seed,source,Stream.of(structureSet));
    }
    static Map<String,Object> capture(long seed,int x,int z,String biome) throws Exception {
        chunks=new HashMap<>(); terrain="air"; surface=96; baseFactory=airFactory;
        Object chunk=chunk(x,z);
        Object manager=make("world.level.StructureManager",null,make("world.level.levelgen.WorldOptions",seed,true,false),null);
        call(generator,"createStructures",registries,structureState,manager,chunk,null,field("world.level.Level","OVERWORLD"));
        Map<String,Object> result=new LinkedHashMap<>(); result.put("seed",seed);result.put("chunk",List.of(x,z));result.put("source",biome);
        Map<?,?> starts=(Map<?,?>)call(chunk,"getAllStarts");
        if(starts.size()>1) throw new IllegalStateException("multiple mineshaft starts in one set");
        List<Object> values=new ArrayList<>();
        Object jsonOps=Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        Object structures=call(registries,"lookupOrThrow",field("core.registries.Registries","STRUCTURE"));
        for(var entry:starts.entrySet()) {
            Object start=entry.getValue();
            List<?> pieces=(List<?>)call(start,"getPieces");
            MessageDigest digest=MessageDigest.getInstance("MD5");
            for(Object piece:pieces) {
                Object tag=call(field("nbt.NbtOps","INSTANCE"),"convertTo",jsonOps,call(piece,"createTag",(Object)null));
                digest.update(canonical(tag).getBytes(StandardCharsets.UTF_8));
            }
            Object bounds=call(start,"getBoundingBox");
            List<Object> bb=new ArrayList<>();
            for(String method:List.of("minX","minY","minZ","maxX","maxY","maxZ")) bb.add(call(bounds,method));
            values.add(Map.of("id",call(structures,"getKey",entry.getKey()).toString(),"bounds",bb,"pieces",pieces.size(),"pieces_md5",HexFormat.of().formatHex(digest.digest())));
        }
        result.put("starts",values);
        return result;
    }
    public static void main(String[] args) throws Exception {
        bootstrapOre(); registries=NativeWorldgenRegistries.load();
        settingsHolder=call(call(registries,"lookupOrThrow",field("core.registries.Registries","NOISE_SETTINGS")),"getOrThrow",field("world.level.levelgen.NoiseGeneratorSettings","OVERWORLD"));
        settings=call(settingsHolder,"value"); noises=call(registries,"lookupOrThrow",field("core.registries.Registries","NOISE"));
        structureSet=call(call(registries,"lookupOrThrow",field("core.registries.Registries","STRUCTURE_SET")),"getOrThrow",key("STRUCTURE_SET","mineshafts"));
        Object placement=call(call(structureSet,"value"),"placement");
        List<Object> samples=new ArrayList<>();
        for(long seed:new long[]{0,1,-1,846692123413862008L,Long.MIN_VALUE,Long.MAX_VALUE}) {
            configure(seed,"overworld");
            List<int[]> candidates=new ArrayList<>();
            for(int x=-32;x<=32;x++) for(int z=-32;z<=32;z++)
                if((boolean)call(placement,"isStructureChunk",structureState,x,z)) candidates.add(new int[]{x,z});
            for(int[] p:candidates) samples.add(capture(seed,p[0],p[1],"overworld"));
            samples.add(capture(seed,0,0,"overworld"));
            if(seed==0 || seed==846692123413862008L) for(String fixed:List.of("plains","badlands","deep_dark")) {
                configure(seed,fixed);
                for(int i=0;i<Math.min(candidates.size(),4);i++) {
                    int[] p=candidates.get(i); samples.add(capture(seed,p[0],p[1],fixed));
                }
            }
        }
        Object structures=call(registries,"lookupOrThrow",field("core.registries.Registries","STRUCTURE"));
        Map<String,Object> result=new LinkedHashMap<>();
        for(String name:List.of("mineshaft","mineshaft_mesa")) {
            Object value=call(call(structures,"getOrThrow",key("STRUCTURE",name)),"value");
            result.put(name+"_biomes",names(call(value,"biomes")));
        }
        List<List<String>> steps=new ArrayList<>();
        for(int i=0;i<11;i++) steps.add(new ArrayList<>());
        for(Object structure:(Iterable<?>)structures) {
            int step=((Enum<?>)call(structure,"step")).ordinal();
            steps.get(step).add(call(structures,"getKey",structure).toString());
        }
        result.put("structure_steps",steps); result.put("samples",samples); result.put("reference_orders",referenceOrders());
        Object gson=Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("MINESHAFT_START_REFERENCE="+call(gson,"toJson",result));
    }
}
