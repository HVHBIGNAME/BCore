import java.lang.reflect.*;
import java.util.*;
import java.util.function.*;
import java.util.jar.*;

/** Real ChunkGenerator start/reference paths; no copied jigsaw/admission algorithm. */
public class StructureRuntimeReference extends JigsawReference {
    static Object randomState, structureState, structureManager, biomeHolder;
    static long worldSeed;
    static int writeSourceX, writeSourceZ;
    static List<String> decorationOrder;
    static Object regionRandom;
    static Object freshLight;

    static List<String> templateNames(String jarPath) throws Exception {
        List<String> names=new ArrayList<>();
        try(JarFile jar=new JarFile(jarPath)) {
            for(JarEntry entry:Collections.list(jar.entries())) {
                String path=entry.getName();
                if(path.startsWith("data/minecraft/structure/") && path.endsWith(".nbt") && (path.contains("/village/")||path.contains("/ancient_city/")))
                    names.add("minecraft:"+path.substring("data/minecraft/structure/".length(),path.length()-4));
            }
        }
        Collections.sort(names); return names;
    }

    static List<Object> templateBlockEntities(String jarPath) throws Exception {
        List<Object> result=new ArrayList<>();
        Object random=make("world.level.levelgen.WorldgenRandom",make("world.level.levelgen.XoroshiroRandomSource",123L));
        Object position=make("core.BlockPos",-4,72,17);
        for(String name:templateNames(jarPath)) {
            Object template=template(name);
            int paletteIndex=0;
            for(Object palette:(List<?>)member(template,"palettes")) {
                int index=0;
                for(Object info:(List<?>)call(palette,"blocks")) {
                    Object state=call(info,"state"),nbt=call(info,"nbt");
                    String blockName=call(blockRegistry,"getKey",call(state,"getBlock")).toString();
                    if(!(boolean)call(state,"hasBlockEntity") || blockName.equals("minecraft:jigsaw") || blockName.equals("minecraft:structure_block")) { index++; continue; }
                    Object entity=call(call(state,"getBlock"),"newBlockEntity",position,state);
                    Object load=nbt==null?make("nbt.CompoundTag"):call(nbt,"copy");
                    if(nbt!=null && type("world.RandomizableContainer").isInstance(entity)) call(load,"putLong","LootTableSeed",call(random,"nextLong"));
                    Object input=call(type("world.level.storage.TagValueInput"),"create",field("util.ProblemReporter","DISCARDING"),registries,load);
                    call(entity,"loadWithComponents",input);
                    result.add(Map.of("template",name,"palette",paletteIndex,"index",index,"state",stateId(state),"pos",xyz(position),
                        "load",Map.of("nbt",nbt64(load)),"full",Map.of("nbt",nbt64(call(entity,"saveWithFullMetadata",registries))),
                        "update",Map.of("nbt",nbt64(call(entity,"getUpdateTag",registries)))));
                    index++;
                }
                paletteIndex++;
            }
        }
        return result;
    }

    static boolean writable(Pos p) {
        return p.y()>=-64 && p.y()<=319 && Math.abs((p.x()>>4)-writeSourceX)<=1 && Math.abs((p.z()>>4)-writeSourceZ)<=1;
    }

    static Object runtimeBlock(Object object) throws Exception {
        Pos p=Pos.from(object);
        if(p.y() < -64 || p.y()>319) return state("AIR");
        Object saved=placed.get(p); if(saved!=null)return saved;
        int distance=Math.max(Math.abs((p.x()>>4)-writeSourceX),Math.abs((p.z()>>4)-writeSourceZ));
        if(distance>8)throw new IllegalStateException("read outside native FEATURES dependencies: "+p);
        if(distance>1)return state("AIR"); // retained STARTS-only outer holders
        if(terrain.equals("city_cave")) return p.y() < -50 || p.y()>=-20?state("STONE"):state("AIR");
        return state(p.y()<64?"STONE":p.y()==64?"GRASS_BLOCK":"AIR");
    }

    static int runtimeHeight(Object kind,int x,int z) throws Exception {
        Predicate<Object> predicate=(Predicate<Object>)call(kind,"isOpaque");
        for(int y=319;y>=-64;y--) if(predicate.test(runtimeBlock(make("core.BlockPos",x,y,z)))) return y+1;
        return -64;
    }

    static Object runtimeWorld() throws Exception {
        return Proxy.newProxyInstance(StructureRuntimeReference.class.getClassLoader(),new Class<?>[]{type("world.level.WorldGenLevel")},(p,m,a)->{
            try {
                return switch(m.getName()) {
                    case "getMinY" -> -64;
                    case "getMaxY" -> 319;
                    case "getMinSectionY" -> -4;
                    case "getSeed" -> worldSeed;
                    case "getRandom" -> regionRandom;
                    case "getHeight" -> a==null||a.length==0?384:runtimeHeight(a[0],(int)a[1],(int)a[2]);
                    case "getHeightmapPos" -> { Pos at=Pos.from(a[1]); yield make("core.BlockPos",at.x(),runtimeHeight(a[0],at.x(),at.z()),at.z()); }
                    case "isOutsideBuildHeight" -> { int y=a[0] instanceof Integer i?i:Pos.from(a[0]).y(); yield y< -64||y>319; }
                    case "ensureCanWrite" -> writable(Pos.from(a[0]));
                    case "getBlockState" -> runtimeBlock(a[0]);
                    case "getFluidState" -> call(runtimeBlock(a[0]),"getFluidState");
                    case "isStateAtPosition" -> ((Predicate<Object>)a[1]).test(runtimeBlock(a[0]));
                    case "isFluidAtPosition" -> ((Predicate<Object>)a[1]).test(call(runtimeBlock(a[0]),"getFluidState"));
                    case "isEmptyBlock" -> call(runtimeBlock(a[0]),"isAir");
                    case "getLightEngine" -> freshLight;
                    case "getSkyDarken" -> 0;
                    case "getMaxLocalRawBrightness", "getRawBrightness" -> InvocationHandler.invokeDefault(p,m,a);
                    case "getBiome" -> biomeHolder;
                    case "isClientSide" -> false;
                    case "registryAccess" -> registries;
                    case "holderLookup" -> call(registries,"lookupOrThrow",a[0]);
                    case "getBlockEntity" -> blockEntities.get(Pos.from(a[0]));
                    case "getChunk" -> {
                        if(a.length==1) { Pos at=Pos.from(a[0]); yield markChunk(at.x()>>4,at.z()>>4); }
                        yield markChunk((int)a[0],(int)a[1]);
                    }
                    case "setBlock" -> {
                        Pos at=Pos.from(a[0]); if(!writable(at)) yield false;
                        Object previous=runtimeBlock(a[0]),block=call(a[1],"getBlock");
                        placed.put(at,a[1]); writes.add(List.of(at.x(),at.y(),at.z(),stateId(a[1]),(int)a[2]));
                        if(!(boolean)call(a[1],"hasBlockEntity")) blockEntities.remove(at);
                        else if(!blockEntities.containsKey(at)||call(previous,"getBlock")!=block) {
                            Object entity=call(block,"newBlockEntity",a[0],a[1]);
                            if(entity!=null)blockEntities.put(at,entity);
                        }
                        if(((int)a[2]&16)==0) {
                            Object post=call(a[1],"getPostProcessPos",p,a[0]);
                            if(post!=null)call(markChunk(at.x()>>4,at.z()>>4),"markPosForPostprocessing",post);
                        }
                        yield true;
                    }
                    case "scheduleTick" -> {
                        boolean fluid=type("world.level.material.Fluid").isInstance(a[1]);
                        Object registry=fluid?field("core.registries.BuiltInRegistries","FLUID"):blockRegistry;
                        ticks.add(List.of(xyz(a[0]),fluid?"fluid":"block",call(registry,"getId",a[1]),a[2])); yield null;
                    }
                    case "setCurrentlyGenerating" -> { if(a[0]!=null)decorationOrder.add(((Supplier<?>)a[0]).get().toString()); yield null; }
                    case "playSound","addParticle","levelEvent","gameEvent" -> null;
                    // This fixture verifies structure BLOCKS and retained effects.
                    // Vanilla's factory catches the missing ServerLevel exception;
                    // no claim about entity creation/finalization comes from it.
                    case "getLevel" -> null;
                    case "addFreshEntityWithPassengers" -> throw new UnsupportedOperationException("live entity unexpectedly created");
                    case "toString" -> "guarded structure block runtime, pending entity factory";
                    default -> throw new UnsupportedOperationException(m.toString());
                };
            } catch(InvocationTargetException e){throw e.getCause();}
        });
    }

    static void resetRuntime(int x,int z) {
        writeSourceX=x;writeSourceZ=z;
        try {
            Object factory=call(randomState,"getOrCreateRandomFactory",identifier("minecraft:worldgen_region_random"));
            regionRandom=call(factory,"at",make("core.BlockPos",x*16,0,z*16));
            Object getter=Proxy.newProxyInstance(StructureRuntimeReference.class.getClassLoader(),new Class<?>[]{type("world.level.chunk.LightChunkGetter")},(p,m,a)->switch(m.getName()) {
                case "getLevel" -> runtimeWorld();
                case "getChunkForLighting" -> markChunk((int)a[0],(int)a[1]);
                case "onLightUpdate" -> throw new IllegalStateException("fresh storage must not propagate");
                default -> throw new UnsupportedOperationException(m.toString());
            });
            freshLight=make("world.level.lighting.LevelLightEngine",getter,true,true);
        } catch(Exception e){throw new RuntimeException(e);}
        placed=new HashMap<>();blockEntities=new HashMap<>();writes=new ArrayList<>();ticks=new ArrayList<>();decorationOrder=new ArrayList<>();
        // Keep starts/references while clearing only marks from the preceding pass.
        for(Object chunk:markChunks.values())try{Arrays.fill((Object[])call(chunk,"getPostProcessing"),null);}catch(Exception e){throw new RuntimeException(e);}
    }

    static Map<String,Object> runtimeSnapshot() throws Exception {
        List<Pos> positions=new ArrayList<>(placed.keySet());
        positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        List<List<Integer>> states=new ArrayList<>();List<Object> entities=new ArrayList<>();
        for(Pos p:positions) {
            states.add(List.of(p.x(),p.y(),p.z(),stateId(placed.get(p))));
            if(blockEntities.containsKey(p))entities.add(Map.of("pos",List.of(p.x(),p.y(),p.z()),
                "full",Map.of("nbt",nbt64(call(blockEntities.get(p),"saveWithFullMetadata",registries))),
                "update",Map.of("nbt",nbt64(call(blockEntities.get(p),"getUpdateTag",registries)))));
        }
        Map<Pos,Integer> marks=new TreeMap<>(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        Method unpack=type("world.level.chunk.ProtoChunk").getMethod("unpackOffsetCoordinates",short.class,int.class,type("world.level.ChunkPos"));
        for(Object chunk:markChunks.values()) {
            Object[] sections=(Object[])call(chunk,"getPostProcessing");
            for(int s=0;s<sections.length;s++)if(sections[s]!=null)for(Object packed:(Iterable<?>)sections[s]) {
                Pos p=Pos.from(unpack.invoke(null,packed,s-4,call(chunk,"getPos")));marks.merge(p,1,Integer::sum);
            }
        }
        Map<String,Object> result=new LinkedHashMap<>();
        result.put("write_count",writes.size());result.put("writes_md5",digest(writes));result.put("state_count",states.size());result.put("states_md5",digest(states));
        result.put("states",states);result.put("block_entities",entities);result.put("ticks",new ArrayList<>(ticks));
        result.put("marks",marks.entrySet().stream().map(e->List.of(e.getKey().x(),e.getKey().y(),e.getKey().z(),e.getValue())).toList());
        return result;
    }

    static List<Object> runtimes() throws Exception {
        List<Object> result=new ArrayList<>();
        for(String biome:List.of("plains","desert","savanna","snowy_plains","taiga","deep_dark")) {
            String set=biome.equals("deep_dark")?"ancient_cities":"villages";
            setup(42,biome,set);terrain=biome.equals("deep_dark")?"city_cave":"surface";
            Object startPos=candidate(set,0,0);int sx=(int)call(startPos,"x"),sz=(int)call(startPos,"z");
            Map<String,Object> admitted=admission(set,biome,sx,sz);
            if(((List<?>)admitted.get("starts")).isEmpty())throw new IllegalStateException("runtime rejected "+biome);
            for(int[] delta:new int[][]{{0,0},{1,0},{-2,1},{3,-2}}) {
                int cx=sx+delta[0],cz=sz+delta[1];Object chunk=markChunk(cx,cz);
                call(generator,"createReferences",referenceWorld(),structureManager,chunk);
                resetRuntime(cx,cz);
                call(generator,"applyBiomeDecoration",runtimeWorld(),chunk,structureManager);
                Map<String,Object> nativeResult=runtimeSnapshot();
                List<String> nativeOrder=new ArrayList<>(decorationOrder);
                // Replay only the native start stream to expose its caller-owned
                // RNG tail; assert it equals the actual applyBiomeDecoration above.
                resetRuntime(cx,cz);
                Object random=make("world.level.levelgen.WorldgenRandom",make("world.level.levelgen.XoroshiroRandomSource",0L));
                long decoration=(long)call(random,"setDecorationSeed",worldSeed,cx*16,cz*16);
                Object section=call(type("core.SectionPos"),"bottomOf",chunk);
                Object clip=call(generator,"getWritableArea",chunk);
                Map<String,Object> tails=new TreeMap<>();
                for(int step=0;step<11;step++) {
                    int index=0;
                    for(Object structure:(Iterable<?>)registry("STRUCTURE")) {
                        if(((Enum<?>)call(structure,"step")).ordinal()!=step)continue;
                        call(random,"setFeatureSeed",decoration,index++,step);
                        for(Object start:(List<?>)call(structureManager,"startsForStructure",section,structure))
                            call(start,"placeInChunk",runtimeWorld(),structureManager,generator,random,clip,make("world.level.ChunkPos",cx,cz));
                        Object bits=member(member(random,"randomSource"),"randomNumberGenerator");
                        Object copy=make("world.level.levelgen.WorldgenRandom",make("world.level.levelgen.XoroshiroRandomSource",member(bits,"seedLo"),member(bits,"seedHi")));
                        tails.put(call(registry("STRUCTURE"),"getKey",structure).toString(),call(copy,"nextLong"));
                    }
                }
                Map<String,Object> replay=runtimeSnapshot();
                if(!nativeResult.equals(replay))throw new IllegalStateException("native decoration/replay differ for "+biome+" "+cx+","+cz);
                Map<String,Object> row=new LinkedHashMap<>(nativeResult);
                row.put("admitted",admitted);row.put("source",List.of(cx,cz));row.put("terrain",terrain);row.put("decoration_seed",decoration);row.put("clip",bounds(clip));
                row.put("decoration_order",nativeOrder);row.put("next_i64",call(random,"nextLong"));
                row.put("structure_rng_tails",tails);
                row.put("fresh_raw_brightness",call(freshLight,"getRawBrightness",make("core.BlockPos",cx*16,64,cz*16),0));
                result.add(row);
            }
        }
        return result;
    }

    static Object holder(String registry, String name) throws Exception {
        return call(registry(registry), "getOrThrow", key(registry, "minecraft:" + name));
    }

    static void setup(long seed, String biome, String set) throws Exception {
        worldSeed = seed;
        Object settings = holder("NOISE_SETTINGS", "overworld");
        randomState = call(type("world.level.levelgen.RandomState"), "create", call(settings, "value"), registry("NOISE"), seed);
        Object source;
        if (biome.equals("overworld")) {
            source = call(type("world.level.biome.MultiNoiseBiomeSource"), "createFromPreset", holder("MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST", "overworld"));
            generator = make("world.level.levelgen.NoiseBasedChunkGenerator", source, settings);
        } else {
            biomeHolder = holder("BIOME", biome);
            Object flat = make("world.level.levelgen.flat.FlatLevelGeneratorSettings", Optional.empty(), biomeHolder, List.of());
            ((List<Object>) call(flat, "getLayersInfo")).add(make("world.level.levelgen.flat.FlatLayerInfo", 129, field("world.level.block.Blocks", "STONE")));
            call(flat, "updateLayers");
            generator = make("world.level.levelgen.FlatLevelSource", flat);
            source = call(generator, "getBiomeSource");
        }
        // Restrict this fixture to one unmodified native set; other structure types
        // are owned by subsequent work and cannot alter this set's selection RNG.
        Constructor<?> constructor = type("world.level.chunk.ChunkGeneratorStructureState").getDeclaredConstructors()[0];
        constructor.setAccessible(true);
        structureState = constructor.newInstance(randomState, source, seed, seed, List.of(holder("STRUCTURE_SET", set)));
        markChunks = new HashMap<>();
        markFactory = call(type("world.level.chunk.PalettedContainerFactory"), "create", registries);
        structureManager = make("world.level.StructureManager", referenceWorld(), make("world.level.levelgen.WorldOptions", seed, true, false), null);
    }

    static Object referenceWorld() throws Exception {
        return Proxy.newProxyInstance(StructureRuntimeReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
            try {
                return switch (m.getName()) {
                    case "getMinY" -> -64;
                    case "getMaxY" -> 319;
                    case "getMinSectionY" -> -4;
                    case "getHeight" -> 384;
                    case "getSeed" -> worldSeed;
                    case "registryAccess" -> registries;
                    case "getChunk" -> markChunk((int)a[0], (int)a[1]);
                    case "toString" -> "StructureRuntimeReference retained proto chunks";
                    default -> throw new UnsupportedOperationException(m.toString());
                };
            } catch (InvocationTargetException e) { throw e.getCause(); }
        });
    }

    static Object candidate(String set, int regionX, int regionZ) throws Exception {
        Object placement = call(call(holder("STRUCTURE_SET", set), "value"), "placement");
        int spacing = (int)call(placement, "spacing");
        return call(placement, "getPotentialStructureChunk", worldSeed, regionX * spacing, regionZ * spacing);
    }

    static Map<String,Object> admission(String set, String biome, int x, int z) throws Exception {
        Object chunk = markChunk(x,z);
        call(generator, "createStructures", registries, structureState, structureManager, chunk, templateManager, field("world.level.Level", "OVERWORLD"));
        List<Object> starts = new ArrayList<>();
        for (Object start : ((Map<?,?>)call(chunk,"getAllStarts")).values()) {
            if (!(boolean)call(start,"isValid")) continue;
            Object structure = call(start,"getStructure");
            String name = call(registry("STRUCTURE"),"getKey",structure).toString();
            Object position = call(start,"getChunkPos");
            starts.add(Map.of("structure",name,"nbt",nbt64(call(start,"createTag",serialContext,position)),
                "reference_bounds",bounds(call(start,"getBoundingBox")),"pieces",((List<?>)call(start,"getPieces")).size()));
        }
        // Re-running createStructures must retain the same object, not rebuild it.
        Map<?,?> before = new HashMap<>((Map<?,?>)call(chunk,"getAllStarts"));
        call(generator, "createStructures", registries, structureState, structureManager, chunk, templateManager, field("world.level.Level", "OVERWORLD"));
        Map<?,?> after = (Map<?,?>)call(chunk,"getAllStarts");
        boolean retained = before.entrySet().stream().allMatch(e -> after.get(e.getKey()) == e.getValue());
        return Map.of("set",set,"seed",worldSeed,"biome",biome,"chunk",List.of(x,z),"starts",starts,"retained",retained);
    }

    static List<Object> admissions() throws Exception {
        List<Object> result = new ArrayList<>();
        for (long seed : new long[]{0,1,42,-17}) {
            for (String biome : List.of("plains","desert","savanna","snowy_plains","taiga","ocean")) {
                setup(seed,biome,"villages");
                Object pos = candidate("villages",seed == -17 ? -2 : 0,seed == 42 ? -1 : 0);
                int x=(int)call(pos,"x"),z=(int)call(pos,"z");
                result.add(admission("villages",biome,x,z));
                result.add(admission("villages",biome,x+1,z));
            }
            for (String biome : List.of("deep_dark","plains")) {
                setup(seed,biome,"ancient_cities");
                Object pos=candidate("ancient_cities",-1,0);
                result.add(admission("ancient_cities",biome,(int)call(pos,"x"),(int)call(pos,"z")));
            }
        }
        // Native noise generator, native climate sampler, real getBaseHeight.
        for (String set : List.of("villages","ancient_cities")) {
            setup(846692123413862008L,"overworld",set);
            int accepted=0;
            for (int radius=0; radius<=10 && accepted<3; radius++) {
                for (int x=-radius;x<=radius && accepted<3;x++) for(int z=-radius;z<=radius && accepted<3;z++) {
                    if (Math.max(Math.abs(x),Math.abs(z))!=radius) continue;
                    Object pos=candidate(set,x,z);
                    Map<String,Object> row=admission(set,"overworld",(int)call(pos,"x"),(int)call(pos,"z"));
                    if(!((List<?>)row.get("starts")).isEmpty()) { result.add(row); accepted++; }
                    else if(radius<=1) result.add(row);
                }
            }
            if(accepted==0) throw new IllegalStateException("No real admitted "+set+" in native search");
        }
        return result;
    }

    static List<Object> references() throws Exception {
        List<Object> result=new ArrayList<>();
        for(String set:List.of("ancient_cities","villages")) {
            String biome=set.equals("villages")?"plains":"deep_dark";
            setup(42,biome,set);
            Object center=candidate(set,0,0);
            int cx=(int)call(center,"x"),cz=(int)call(center,"z");
            List<Object> sources=new ArrayList<>();
            for(int x=cx-9;x<=cx+9;x++) for(int z=cz-9;z<=cz+9;z++) {
                Map<String,Object> row=admission(set,biome,x,z);
                if(!((List<?>)row.get("starts")).isEmpty()) sources.add(row);
            }
            List<Object> targets=new ArrayList<>();
            for(int dx=-7;dx<=7;dx++) for(int dz=-7;dz<=7;dz++) {
                int x=cx+dx,z=cz+dz; Object target=markChunk(x,z);
                call(generator,"createReferences",referenceWorld(),structureManager,target);
                Map<String,Object> refs=new TreeMap<>();
                for(var entry:((Map<?,?>)call(target,"getAllReferences")).entrySet()) {
                    List<Object> list=new ArrayList<>();
                    for(Object packed:(Iterable<?>)entry.getValue()) {
                        Object pos=call(type("world.level.ChunkPos"),"unpack",packed);
                        list.add(List.of(call(pos,"x"),call(pos,"z")));
                    }
                    refs.put(call(registry("STRUCTURE"),"getKey",entry.getKey()).toString(),list);
                }
                targets.add(Map.of("chunk",List.of(x,z),"references",refs));
            }
            result.add(Map.of("set",set,"seed",42,"biome",biome,"sources",sources,"targets",targets));
        }
        return result;
    }

    static List<Object> metadata() throws Exception {
        Map<Integer,Integer> indices=new HashMap<>(); List<Object> rows=new ArrayList<>();
        for(Object value:(Iterable<?>)registry("STRUCTURE")) {
            int step=((Enum<?>)call(value,"step")).ordinal();
            int index=indices.getOrDefault(step,0); indices.put(step,index+1);
            rows.add(Map.of("structure",call(registry("STRUCTURE"),"getKey",value).toString(),"step",step,"index",index));
        }
        return rows;
    }

    static List<Object> blockEntityPolicies() throws Exception {
        List<Object> result=new ArrayList<>();
        for(Object block:(Iterable<?>)blockRegistry) {
            Object state=call(block,"defaultBlockState");
            if(!(boolean)call(state,"hasBlockEntity")) continue;
            Object entity=call(block,"newBlockEntity",make("core.BlockPos",1,70,-2),state);
            if(entity==null) continue;
            Object update=call(entity,"getUpdateTag",registries);
            result.add(Map.of("block",call(blockRegistry,"getKey",block).toString(),"type_id",call(field("core.registries.BuiltInRegistries","BLOCK_ENTITY_TYPE"),"getId",call(entity,"getType")),
                "full",Map.of("nbt",nbt64(call(entity,"saveWithFullMetadata",registries))),"update",Map.of("nbt",nbt64(update)),
                "update_owner",entity.getClass().getMethod("getUpdateTag",type("core.HolderLookup$Provider")).getDeclaringClass().getName()));
        }
        return result;
    }

    static List<Object> randomStream() throws Exception {
        Object random=make("world.level.levelgen.WorldgenRandom",make("world.level.levelgen.XoroshiroRandomSource",0L));
        long decoration=(long)call(random,"setDecorationSeed",-17L,240,-192);
        List<Object> result=new ArrayList<>();
        result.add(Map.of("op","decoration","seed",-17L,"x",240,"z",-192,"value",decoration));
        result.add(Map.of("op","gaussian","bits",Long.toUnsignedString(Double.doubleToRawLongBits((double)call(random,"nextGaussian")))));
        for(int[] pair:new int[][]{{22,4},{0,7},{3,7},{17,9}}) {
            call(random,"setFeatureSeed",decoration,pair[0],pair[1]);
            result.add(Map.of("op","feature","index",pair[0],"step",pair[1]));
            result.add(Map.of("op","gaussian","bits",Long.toUnsignedString(Double.doubleToRawLongBits((double)call(random,"nextGaussian")))));
            result.add(Map.of("op","long","value",call(random,"nextLong")));
        }
        return result;
    }

    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries(); configureFlat();
        String section=args.length>1?args[1]:"all";
        Map<String,Object> data=new LinkedHashMap<>();
        data.put("scope","Native ChunkGenerator.createStructures/createReferences on retained ProtoChunks, filtered native sets; fixed-biome flat and real overworld-noise cases. Entity factories and live server scheduling are not invoked by these sections.");
        if(section.equals("all")||section.equals("admission")) { data.put("admission",admissions()); data.put("references",references()); data.put("metadata",metadata()); }
        if(section.equals("all")||section.equals("policies")) { data.put("block_entity_policies",blockEntityPolicies()); data.put("template_block_entities",templateBlockEntities(args[0])); }
        if(section.equals("all")||section.equals("runtime")) { data.put("runtime",runtimes()); data.put("random_stream",randomStream()); }
        output("STRUCTURERUNTIMEREFERENCE",data); call(resources,"close");
    }
}
