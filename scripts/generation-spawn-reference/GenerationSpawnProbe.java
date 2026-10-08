import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;
import java.util.function.*;
import java.util.stream.*;

/** Calls native spawning, native block rules, native factories and finalizers. */
public final class GenerationSpawnProbe {
    static Object gson, level, registries, biomeRegistry, entityRegistry;
    static Map<?, ?> config;
    static Path output;
    static final List<String> instrumented = new CopyOnWriteArrayList<>();
    static final List<Object> rows = new CopyOnWriteArrayList<>();
    static final ThreadLocal<Case> ACTIVE = new ThreadLocal<>();
    static boolean running;

    static Class<?> type(String name) throws Exception { return NativeAccess.type(name); }
    static Object call(Object target, String method, Object... args) throws Exception { return NativeAccess.call(target, method, args); }
    static Object make(String owner, Object... args) throws Exception { return NativeAccess.make(owner, args); }
    static Object constant(String owner, String field) throws Exception { return NativeAccess.constant(owner, field); }
    static Object field(Object target, String field) throws Exception { return NativeAccess.field(target, field); }
    static Map<String,Object> map(Object... pairs) { return NativeAccess.map(pairs); }
    static Object identifier(String name) throws Exception { return call(type("resources.Identifier"), "withDefaultNamespace", name.replace("minecraft:", "")); }
    static Object registry(String key) throws Exception { return call(registries, "lookupOrThrow", constant("core.registries.Registries", key)); }
    static Object entity(String key) throws Exception { return call(entityRegistry, "getValue", identifier(key)); }
    static Object biome(String key) throws Exception { return ((Optional<?>)call(biomeRegistry, "get", identifier(key))).orElseThrow(); }
    static Object block(String key) throws Exception { return call(call(constant("core.registries.BuiltInRegistries", "BLOCK"), "getValue", identifier(key)), "defaultBlockState"); }
    static Object pos(int x, int y, int z) throws Exception { return make("core.BlockPos", x, y, z); }
    static int[] xyz(Object pos) throws Exception { return new int[]{(int)call(pos,"getX"),(int)call(pos,"getY"),(int)call(pos,"getZ")}; }
    static int stateId(Object state) throws Exception { return (int)call(type("world.level.block.Block"), "getId", state); }
    static String entityName(Object value) throws Exception { return call(entityRegistry,"getKey",value).toString(); }
    static String holderName(Object holder) throws Exception { return call(((Optional<?>)call(holder,"unwrapKey")).orElseThrow(),"identifier").toString(); }

    static void initialize(String path) throws Exception {
        Object builder=Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
        gson = call(call(builder,"setPrettyPrinting"),"create");
        config = (Map<?, ?>)call(gson,"fromJson",Files.readString(Path.of(path)),Map.class);
        output = Path.of((String)config.get("output"));
    }
    static void instrumented(String name) { instrumented.add(name); }
    public static void fatal(Throwable failure) {
        failure.printStackTrace();
        Runtime.getRuntime().halt(3);
    }

    static final class Rng implements InvocationHandler {
        final Object source;
        final Object proxy;
        final String name;
        final List<String> draws = new ArrayList<>();
        Rng(String name, Object source) throws Exception {
            this.name=name; this.source=source;
            proxy=Proxy.newProxyInstance(GenerationSpawnProbe.class.getClassLoader(),new Class<?>[]{type("util.RandomSource")},this);
        }
        @Override public Object invoke(Object proxy, Method method, Object[] args) throws Throwable {
            if (method.getDeclaringClass()==Object.class) return method.invoke(this,args);
            if (method.isDefault()) return InvocationHandler.invokeDefault(proxy,method,args==null?new Object[0]:args);
            Object result;
            try { result=method.invoke(source,args); }
            catch(InvocationTargetException e) { throw e.getCause(); }
            String draw=switch(method.getName()) {
                case "nextInt" -> "i:"+(args==null?0:args[0])+":"+result;
                case "nextLong" -> "l:"+result;
                case "nextFloat" -> "f:"+Integer.toUnsignedString(Float.floatToRawIntBits((float)result));
                case "nextDouble" -> "d:"+Long.toUnsignedString(Double.doubleToRawLongBits((double)result));
                case "nextBoolean" -> "b:"+result;
                case "nextGaussian" -> "g:"+Long.toUnsignedString(Double.doubleToRawLongBits((double)result));
                default -> throw new UnsupportedOperationException("Unexpected native RNG API "+method);
            };
            draws.add(draw);
            return result;
        }
        Map<String,Object> finish() throws Exception {
            return map("stream",name,"draws",List.copyOf(draws),"next_i64",call(source,"nextLong").toString());
        }
    }

    static final class Case implements InvocationHandler {
        final Map<String,Object> input;
        final List<Object> events=new ArrayList<>(), entities=new ArrayList<>();
        final List<Rng> entityRngs=new ArrayList<>();
        final Object biome;
        final Rng environment;
        final Object border;
        Rng placement;
        final Object world;
        final Object nativeWorld;
        final Map<String,Object> reads=new TreeMap<>();
        int finalDepth;
        Case(Map<String,Object> input) throws Exception {
            this(input,null);
        }
        Case(Map<String,Object> input,Object nativeWorld) throws Exception {
            this.input=input;
            this.nativeWorld=nativeWorld;
            biome=biome((String)input.get("biome"));
            environment=new Rng("environment",nativeWorld==null ? make("world.level.levelgen.XoroshiroRandomSource",Long.parseLong((String)input.get("environment_seed"))) : call(nativeWorld,"getRandom"));
            border=nativeWorld==null ? make("world.level.border.WorldBorder") : call(nativeWorld,"getWorldBorder");
            if(input.containsKey("border_size")) call(border,"setSize",((Number)input.get("border_size")).doubleValue());
            world=Proxy.newProxyInstance(GenerationSpawnProbe.class.getClassLoader(),new Class<?>[]{type("world.level.WorldGenLevel")},this);
        }
        Object get(Object p) throws Exception {
            int[] v=xyz(p);
            int ground=((Number)input.get("ground_y")).intValue();
            String terrain=(String)input.get("terrain");
            String material=(String)input.get("ground");
            if(v[1]<-64 || v[1]>319) return block("air");
            if(terrain.equals("void")) return block("air");
            if(terrain.equals("steps")) ground+=Math.floorMod(v[0],4);
            if(v[1]<=ground) return block(material);
            if(v[1]==ground+1) {
                if(terrain.equals("water")) return block("water");
                if(terrain.equals("snow")) return block("snow");
                if(terrain.equals("fence")) return block("oak_fence");
            }
            if(terrain.equals("low_ceiling") && v[1]==ground+2) return block("stone");
            if(terrain.equals("lateral_collision") && Math.floorMod(v[0],2)==0 && v[1]<=ground+3) return block("stone");
            return block("air");
        }
        int height(Object kind,int x,int z) throws Exception {
            @SuppressWarnings("unchecked") Predicate<Object> opaque=(Predicate<Object>)call(kind,"isOpaque");
            for(int y=319;y>=-64;y--) if(opaque.test(get(pos(x,y,z)))) return y+1;
            return -64;
        }
        @Override public Object invoke(Object p,Method m,Object[] a) throws Throwable {
            if(nativeWorld!=null) return nativeInput(m,a);
            switch(m.getName()) {
                case "getLevel": return level;
                case "registryAccess": return registries;
                case "enabledFeatures": return call(level,"enabledFeatures");
                case "getRandom": return environment.proxy;
                case "getBiome": return biome;
                case "getUncachedNoiseBiome": return biome;
                case "getMinY": return -64;
                case "getHeight":
                    if(a==null || a.length==0) return 384;
                    int h=height(a[0],(int)a[1],(int)a[2]);
                    events.add(map("height",a[0].toString(),"x",a[1],"z",a[2],"y",h));
                    return h;
                case "getBlockState": return get(a[0]);
                case "getFluidState": return call(get(a[0]),"getFluidState");
                case "getBlockEntity": return null;
                case "getChunkForCollisions": return p;
                case "getWorldBorder": return border;
                case "getRawBrightness": return ((Number)input.get("brightness")).intValue();
                case "getSkyDarken": return 0;
                case "getSeaLevel": return 63;
                case "getSeed": return Long.parseLong((String)input.get("seed"));
                case "dimensionType": return call(level,"dimensionType");
                case "environmentAttributes": return call(level,"environmentAttributes");
                case "getLevelData": return call(level,"getLevelData");
                case "getServer": return call(level,"getServer");
                case "getCurrentDifficultyAt": return make("world.DifficultyInstance",constant("world.Difficulty","NORMAL"),0L,0L,0.0f);
                case "isClientSide": return false;
                case "getEntities": case "getEntityCollisions": case "players": return List.of();
                case "addFreshEntity":
                    entities.add(saveEntity(a[0]));
                    return true;
                case "isUnobstructed":
                    if(a.length==2 && type("world.phys.shapes.VoxelShape").isInstance(a[1]))
                        return !input.getOrDefault("obstructed",false).equals(true);
                    break;
                case "noCollision":
                    if(input.getOrDefault("collision",false).equals(true)) return false;
                    break;
                case "toString": return "GenerationSpawnControlledWorld";
            }
            if(m.isDefault()) return InvocationHandler.invokeDefault(p,m,a==null?new Object[0]:a);
            throw new UnsupportedOperationException("Unprovided native world input "+m);
        }
        Object nativeInput(Method method,Object[] args) throws Throwable {
            if(method.getName().equals("getRandom")) return environment.proxy;
            if(method.getName().equals("addFreshEntityWithPassengers"))
                return InvocationHandler.invokeDefault(world,method,args);
            if(method.getName().equals("addFreshEntity")) entities.add(saveEntity(args[0]));
            Object result;
            try { result=method.invoke(nativeWorld,args); }
            catch(InvocationTargetException error) { throw error.getCause(); }
            String name=method.getName();
            if(name.equals("getBlockState")) reads.put("block:"+Arrays.toString(xyz(args[0])),stateId(result));
            else if(name.equals("getBiome")) reads.put("biome:"+Arrays.toString(xyz(args[0])),holderName(result));
            else if(name.equals("getHeight") && args!=null && args.length==3) {
                events.add(map("height",args[0].toString(),"x",args[1],"z",args[2],"y",result));
                reads.put("height:"+args[0]+":"+args[1]+":"+args[2],result);
            } else if(name.equals("getRawBrightness")) reads.put("brightness:"+Arrays.toString(xyz(args[0]))+":"+args[1],result);
            else if(name.equals("getPathfindingCostFromLightLevels")) reads.put("path_cost:"+Arrays.toString(xyz(args[0])),result);
            else if(name.equals("noCollision") && args.length==1) reads.put("collision:"+Arrays.toString(box(args[0])),result);
            else if(name.equals("containsAnyLiquid")) reads.put("liquid:"+Arrays.toString(box(args[0])),result);
            else if(name.equals("isUnobstructed") && args.length==1) reads.put("unobstructed:"+Arrays.toString(box(call(args[0],"getBoundingBox"))),result);
            else if(name.equals("getCurrentDifficultyAt")) reads.put("difficulty:"+Arrays.toString(xyz(args[0])),
                map("difficulty",call(call(result,"getDifficulty"),"getId"),"effective",call(result,"getEffectiveDifficulty")));
            return result;
        }
        Map<String,Object> finish() throws Exception {
            List<Object> streams=new ArrayList<>();
            if(placement!=null) streams.add(placement.finish());
            streams.add(environment.finish());
            for(Rng rng:entityRngs) streams.add(rng.finish());
            return map("input",input,"entities",entities,"events",events,"rng",streams,"reads",reads);
        }
    }

    public static Object worldArgument(Object world) { Case c=ACTIVE.get(); return c!=null && c.nativeWorld!=null ? c.world : world; }
    public static Object placementArgument(Object rng) throws Exception {
        Case c=ACTIVE.get();
        if(c!=null && c.nativeWorld!=null) { c.placement=new Rng("placement",rng); return c.placement.proxy; }
        return rng;
    }

    public static Object entityRandom() throws Exception {
        Case c=ACTIVE.get();
        if(c==null) return call(type("util.RandomSource"),"create");
        int index=c.entityRngs.size();
        long seed=Long.parseLong((String)c.input.get("entity_seed"))+index;
        Rng rng=new Rng("entity:"+index,make("world.level.levelgen.LegacyRandomSource",seed));
        c.entityRngs.add(rng);
        c.events.add(map("entity_entropy",index,"seed",Long.toString(seed)));
        return rng.proxy;
    }

    static Object typedNbt(Object tag) throws Exception {
        int id=((Number)call(tag,"getId")).intValue();
        Object payload;
        if(id==10) {
            Map<String,Object> children=new TreeMap<>();
            for(Object entry:(Set<?>)call(tag,"entrySet")) {
                Map.Entry<?,?> pair=(Map.Entry<?,?>)entry;
                children.put((String)pair.getKey(),typedNbt(pair.getValue()));
            }
            payload=children;
        } else if(id==9) {
            List<Object> children=new ArrayList<>();
            for(Object child:(List<?>)tag) children.add(typedNbt(child));
            payload=children;
        } else payload=switch(id) {
            case 1,2,3,4,5,6 -> call(tag,"box");
            case 7 -> call(tag,"getAsByteArray");
            case 8 -> call(tag,"value");
            case 11 -> call(tag,"getAsIntArray");
            case 12 -> call(tag,"getAsLongArray");
            default -> throw new IllegalStateException("NBT type "+id);
        };
        return List.of(id,payload);
    }
    static Map<String,Object> saveEntity(Object entity) throws Exception {
        Object problems=make("util.ProblemReporter$Collector");
        Object out=call(type("world.level.storage.TagValueOutput"),"createWithContext",problems,registries);
        if(!(boolean)call(entity,"save",out)) throw new IllegalStateException("Native entity refused save");
        Object tag=call(out,"buildResult");
        ByteArrayOutputStream bytes=new ByteArrayOutputStream();
        call(type("nbt.NbtIo"),"write",tag,new DataOutputStream(bytes));
        Map<String,Object> result=map("type",entityName(call(entity,"getType")),"typed_nbt",typedNbt(tag),
            "nbt_hex",HexFormat.of().formatHex(bytes.toByteArray()),"problems",call(problems,"getReport"),
            "position",List.of(call(entity,"getX"),call(entity,"getY"),call(entity,"getZ")),
            "rotation",List.of(call(entity,"getYRot"),call(entity,"getXRot")),
            "head_yaw",field(entity,"yHeadRot"),"sensors",sensors(entity),"is_baby",call(entity,"isBaby"));
        if(config.get("mode").equals("handoff") && !GenerationSpawnHandoff.observing)
            result.put("handoff",GenerationSpawnHandoff.observe(entity,tag));
        return result;
    }
    static List<Object> sensors(Object entity) throws Exception {
        List<Object> result=new ArrayList<>();
        Object registry=constant("core.registries.BuiltInRegistries","SENSOR_TYPE");
        Map<?,?> sensors=(Map<?,?>)field(call(entity,"getBrain"),"sensors");
        for(var entry:sensors.entrySet()) result.add(map("name",call(registry,"getKey",entry.getKey()).toString(),
            "scan_rate",field(entry.getValue(),"scanRate"),"time_to_tick",field(entry.getValue(),"timeToTick")));
        return result;
    }
    static Object group(Object data) throws Exception {
        if(data==null) return null;
        Map<String,Object> result=map("class",data.getClass().getName());
        if(type("world.entity.AgeableMob$AgeableMobGroupData").isInstance(data)) {
            result.put("size",call(data,"getGroupSize"));
            result.put("babies",call(data,"isShouldSpawnBaby"));
            result.put("baby_chance",call(data,"getBabySpawnChance"));
        }
        return result;
    }
    public static void enter(String method,Object self,Object[] args) {
        try {
            if(method.equals("server/MinecraftServer.setInitialSpawn") && !running) {
                running=true;
                run(args[0]);
                return;
            }
            if(method.equals("server/MinecraftServer.tickServer")) throw new IllegalStateException("Gameplay tick entered");
            if(method.equals("world/level/NaturalSpawner.spawnMobsForChunkGeneration") && config.get("mode").equals("entry")) {
                int x=(int)call(args[2],"x"), z=(int)call(args[2],"z");
                Map<String,Object> input=map("name","entry-"+x+"-"+z,"biome",holderName(args[1]),
                    "seed",call(args[0],"getSeed").toString(),"chunk",List.of(x,z),"entity_seed","1000",
                    "entry_biome_pos",List.of(x<<4,call(args[0],"getMaxY"),z<<4),
                    "entry_path","ChunkStatusTasks.generateSpawn/NoiseBasedChunkGenerator.spawnOriginalMobs",
                    "environment_rng","WorldGenRegion native named positional random");
                Case c=new Case(input,args[0]);
                Object bits=field(c.environment.source,"randomNumberGenerator");
                input.put("environment_state",List.of(field(bits,"seedLo").toString(),field(bits,"seedHi").toString()));
                ACTIVE.set(c);
            }
            Case c=ACTIVE.get(); if(c==null) return;
            if(method.endsWith(".finalizeSpawn")) {
                if(c.finalDepth++==0) c.events.add(map("finalize_enter",entityName(call(self,"getType")),"group",group(args[3]),"reason",args[2].toString()));
            } else if(method.equals("world/entity/EntityType.create") && args.length==2) {
                c.events.add(map("factory",entityName(self),"reason",args[1].toString()));
            }
        } catch(Throwable failure) { fatal(failure); }
    }
    public static void exit(Object result,String method,Object self,Object[] args) {
        try {
            Case c=ACTIVE.get(); if(c==null) return;
            if(method.equals("world/level/NaturalSpawner.spawnMobsForChunkGeneration") && c.nativeWorld!=null) {
                rows.add(c.finish()); ACTIVE.remove(); return;
            }
            if(method.endsWith(".finalizeSpawn")) {
                if(--c.finalDepth==0) c.events.add(map("finalize_exit",entityName(call(self,"getType")),"group",group(result)));
            } else if(method.equals("world/level/NaturalSpawner.getTopNonCollidingPos")) {
                c.events.add(map("top",xyz(result),"type",entityName(args[1])));
            } else if(method.equals("world/entity/SpawnPlacements.isSpawnPositionOk")) {
                c.events.add(map("placement_ok",result,"pos",xyz(args[2]),"type",entityName(args[0])));
            } else if(method.equals("world/entity/SpawnPlacements.checkSpawnRules")) {
                c.events.add(map("static_rule",result,"pos",xyz(args[3]),"type",entityName(args[0]),"reason",args[2].toString()));
            } else if(method.endsWith(".checkSpawnRules") && self!=null) {
                c.events.add(map("mob_rule",result,"type",entityName(call(self,"getType"))));
            } else if(method.endsWith(".checkSpawnObstruction") && self!=null) {
                c.events.add(map("obstruction_ok",result,"type",entityName(call(self,"getType"))));
            }
        } catch(Throwable failure) { fatal(failure); }
    }

    static Object forcedBiome(Object base,String name,int min,int max,float probability) throws Exception {
        Object settings=make("world.level.biome.MobSpawnSettings$Builder");
        call(settings,"creatureGenerationProbability",probability);
        if(!name.equals("empty")) call(settings,"addSpawn",constant("world.entity.MobCategory","CREATURE"),1,
            make("world.level.biome.MobSpawnSettings$SpawnerData",entity(name),min,max));
        Object builder=make("world.level.biome.Biome$BiomeBuilder");
        call(builder,"temperature",0.8f); call(builder,"downfall",0.4f);
        call(builder,"specialEffects",call(call(base,"value"),"getSpecialEffects"));
        call(builder,"generationSettings",call(call(base,"value"),"getGenerationSettings"));
        call(builder,"mobSpawnSettings",call(settings,"build"));
        return call(type("core.Holder"),"direct",call(builder,"build"));
    }
    static Map<String,Object> input(String name,String biome,long seed,int x,int z,String forced) {
        Map<String,Object> row=map("name",name,"biome",biome,"seed",Long.toString(seed),"chunk",List.of(x,z),
            "environment_seed","17","entity_seed","1000","ground","grass_block","ground_y",64,
            "terrain","flat","brightness",15);
        if(forced!=null) { row.put("forced",forced); row.put("probability",0.45f); row.put("min",2); row.put("max",4); }
        return row;
    }
    static void sample(Map<String,Object> input) throws Exception {
        Object levelData=call(level,"getLevelData");
        long oldTime=(long)call(levelData,"getGameTime");
        if(input.containsKey("game_time")) call(levelData,"setGameTime",Long.parseLong((String)input.get("game_time")));
        else input.put("game_time",Long.toString(oldTime));
        Case c=new Case(input);
        ACTIVE.set(c);
        try {
            @SuppressWarnings("unchecked") List<Number> chunk=(List<Number>)input.get("chunk");
            int x=chunk.get(0).intValue(), z=chunk.get(1).intValue();
            Object rng=make("world.level.levelgen.WorldgenRandom",make("world.level.levelgen.LegacyRandomSource",0L));
            long decoration=(long)call(rng,"setDecorationSeed",Long.parseLong((String)input.get("seed")),x<<4,z<<4);
            input.put("decoration_seed",Long.toString(decoration));
            c.placement=new Rng("placement",rng);
            Object selected=input.containsKey("forced") ? forcedBiome(c.biome,(String)input.get("forced"),
                ((Number)input.get("min")).intValue(),((Number)input.get("max")).intValue(),((Number)input.get("probability")).floatValue()) : c.biome;
            call(type("world.level.NaturalSpawner"),"spawnMobsForChunkGeneration",c.world,selected,make("world.level.ChunkPos",x,z),c.placement.proxy);
            rows.add(c.finish());
        } finally { ACTIVE.remove(); call(levelData,"setGameTime",oldTime); }
    }

    static List<Object> spawnList(Object settings) throws Exception {
        Object list=call(settings,"getMobs",constant("world.entity.MobCategory","CREATURE"));
        List<Object> out=new ArrayList<>();
        for(Object entry:(List<?>)call(list,"unwrap")) {
            Object data=call(entry,"value");
            out.add(map("type",entityName(call(data,"type")),"weight",call(entry,"weight"),"min",call(data,"minCount"),"max",call(data,"maxCount")));
        }
        return out;
    }
    static Object catalog() throws Exception {
        List<Object> biomes=new ArrayList<>();
        SortedSet<String> entityNames=new TreeSet<>();
        for(Object b:(Iterable<?>)biomeRegistry) {
            Object holder=call(biomeRegistry,"wrapAsHolder",b);
            Object settings=call(b,"getMobSettings");
            List<Object> list=spawnList(settings);
            for(Object entry:list) entityNames.add((String)((Map<?,?>)entry).get("type"));
            Map<String,Object> row=map("id",call(biomeRegistry,"getId",b),"name",call(biomeRegistry,"getKey",b).toString(),
                "probability",call(settings,"getCreatureProbability"),"spawns",list);
            List<String> tags=new ArrayList<>();
            try(Stream<?> stream=(Stream<?>)call(holder,"tags")) {
                for(Object tag:stream.toList()) tags.add(call(tag,"location").toString());
            }
            Collections.sort(tags); row.put("tags",tags); biomes.add(row);
        }
        List<Object> types=new ArrayList<>();
        for(String name:entityNames) {
            Object t=entity(name);
            Object placement=call(type("world.entity.SpawnPlacements"),"getPlacementType",t);
            String placementName="unknown";
            for(String n:List.of("ON_GROUND","NO_RESTRICTIONS","IN_WATER","IN_LAVA"))
                if(placement==constant("world.entity.SpawnPlacementTypes",n)) placementName=n;
            Object defaults=call(type("world.entity.ai.attributes.DefaultAttributes"),"getSupplier",t);
            types.add(map("name",name,"id",call(entityRegistry,"getId",t),"width",call(t,"getWidth"),"height",call(t,"getHeight"),
                "spawn_box",box(call(t,"getSpawnAABB",0.0,0.0,0.0)),"summon",call(t,"canSummon"),
                "heightmap",call(type("world.entity.SpawnPlacements"),"getHeightmapType",t).toString(),"placement",placementName,
                "follow_range",call(defaults,"getBaseValue",constant("world.entity.ai.attributes.Attributes","FOLLOW_RANGE"))));
        }
        Map<String,Object> sounds=new TreeMap<>();
        for(String key:List.of("COW_SOUND_VARIANT","PIG_SOUND_VARIANT","CHICKEN_SOUND_VARIANT","WOLF_SOUND_VARIANT")) {
            Object r=registry(key); List<String> names=new ArrayList<>();
            for(Object v:(Iterable<?>)r) names.add(call(r,"getKey",v).toString());
            sounds.put(key.toLowerCase(Locale.ROOT),names);
        }
        Map<String,Object> variants=new TreeMap<>();
        for(String key:List.of("COW_VARIANT","PIG_VARIANT","CHICKEN_VARIANT","WOLF_VARIANT","FROG_VARIANT")) {
            Object r=registry(key); Map<String,Object> byBiome=new TreeMap<>();
            for(Object row:biomes) {
                String name=(String)((Map<?,?>)row).get("name");
                Case c=new Case(input("variant-candidates",name,0,0,0,null));
                Object context=call(type("world.entity.variant.SpawnContext"),"create",c.world,pos(0,65,0));
                Function<Object,Object> value=holder -> { try { return call(holder,"value"); } catch(Exception e) { throw new RuntimeException(e); } };
                List<String> names=new ArrayList<>();
                try(Stream<?> selected=(Stream<?>)call(type("world.entity.variant.PriorityProvider"),"select",call(r,"listElements"),value,context)) {
                    for(Object choice:selected.toList()) names.add(holderName(choice));
                }
                byBiome.put(name,names);
            }
            variants.put(key.toLowerCase(Locale.ROOT),byBiome);
        }
        return map("biomes",biomes,"types",types,"sound_variants",sounds,"variant_candidates",variants);
    }
    static List<List<Double>> boxes(Object shape) throws Exception {
        List<List<Double>> boxes=new ArrayList<>();
        for(Object b:(List<?>)call(shape,"toAabbs")) boxes.add(Arrays.stream(box(b)).boxed().toList());
        return boxes;
    }
    static Object blockMetadata(List<?> types) throws Exception {
        Case empty=new Case(input("state-input","plains",0,0,0,null));
        empty.input.put("terrain","void");
        Object zero=pos(0,0,0), shifted=pos(17,71,-11);
        Object land=constant("world.level.pathfinder.PathComputationType","LAND");
        List<Object> entities=new ArrayList<>();
        for(Object item:types) entities.add(entity((String)((Map<?,?>)item).get("name")));
        Map<List<List<Double>>,Integer> shapeIds=new HashMap<>();
        List<Object> shapes=new ArrayList<>(), runs=new ArrayList<>(), shapeSamples=new ArrayList<>();
        Map<String,Object> offsets=new TreeMap<>();
        List<Long> previous=null;
        int start=0, count=0;
        List<Integer> contextual=new ArrayList<>();
        for(Object state:(Iterable<?>)constant("world.level.block.Block","BLOCK_STATE_REGISTRY")) {
            int id=stateId(state); if(id!=count++) throw new IllegalStateException("Unordered states");
            List<List<Double>> shape;
            boolean dynamic=false, offsetShape=false, full=false;
            try {
                Object nativeShape=call(state,"getCollisionShape",empty.world,zero);
                full=nativeShape==call(type("world.phys.shapes.Shapes"),"block");
                shape=boxes(nativeShape);
                dynamic=!shape.equals(boxes(call(state,"getCollisionShape",empty.world,shifted)));
                if(dynamic && (boolean)call(state,"hasOffsetFunction")) {
                    Object offset=call(state,"getOffset",zero);
                    Object b=call(state,"getBlock");
                    float horizontal=(float)call(b,"getMaxHorizontalOffset");
                    float vertical=(double)field(offset,"y")==0.0 ? 0.0f : (float)call(b,"getMaxVerticalOffset");
                    shape=boxes(call(nativeShape,"move",-(double)field(offset,"x"),-(double)field(offset,"y"),-(double)field(offset,"z")));
                    offsets.put(Integer.toString(id),List.of(horizontal,vertical));
                    offsetShape=true; dynamic=false;
                    for(int[] p:new int[][]{{0,0,0},{17,71,-11},{-16,319,-16},{30000000,-64,29999999}})
                        shapeSamples.add(map("state",id,"pos",p,"boxes",boxes(call(state,"getCollisionShape",empty.world,pos(p[0],p[1],p[2])))));
                }
            } catch(Exception failure) {
                shape=List.of(); dynamic=true; contextual.add(id);
            }
            Integer shapeId=shapeIds.get(shape);
            if(shapeId==null) { shapeId=shapes.size(); shapeIds.put(shape,shapeId); shapes.add(shape); }
            long floor=0,inside=0;
            for(int i=0;i<entities.size();i++) {
                if((boolean)call(state,"isValidSpawn",empty.world,zero,entities.get(i))) floor|=1L<<i;
                if((boolean)call(type("world.level.NaturalSpawner"),"isValidEmptySpawnBlock",empty.world,zero,state,call(state,"getFluidState"),entities.get(i))) inside|=1L<<i;
            }
            int flags=((boolean)call(state,"isPathfindable",land)?1:0)
                | ((boolean)call(state,"hasLargeCollisionShape")?2:0)
                | (dynamic?4:0) | (offsetShape?8:0) | (full?16:0);
            List<Long> values=List.of((long)shapeId, (long)flags, floor, inside);
            if(previous!=null && !previous.equals(values)) {
                List<Long> row=new ArrayList<>(List.of((long)start,(long)id)); row.addAll(previous); runs.add(row); start=id;
            }
            previous=values;
        }
        List<Long> row=new ArrayList<>(List.of((long)start,(long)count)); row.addAll(previous); runs.add(row);
        return map("state_count",count,"shapes",shapes,"runs",runs,"contextual_states",contextual,
            "offsets",offsets,"shape_samples",shapeSamples,
            "columns",List.of("first","end","shape","flags:land_pathfindable=1,large_shape=2,dynamic_shape=4,offset_shape=8,canonical_full=16","valid_spawn_mask","valid_empty_mask"));
    }
    static Object templates(List<?> types) throws Exception {
        Map<String,Object> out=new TreeMap<>();
        for(Object item:types) {
            String name=(String)((Map<?,?>)item).get("name");
            Case c=new Case(input("constructor-"+name,"plains",0,0,0,null));
            ACTIVE.set(c);
            try {
                Object value=call(entity(name),"create",level,constant("world.entity.EntitySpawnReason","NATURAL"));
                out.put(name,map("entity",saveEntity(value),"rng",c.entityRngs.get(0).finish()));
            } finally { ACTIVE.remove(); }
        }
        return out;
    }
    static double[] box(Object box) throws Exception {
        return new double[]{(double)field(box,"minX"),(double)field(box,"minY"),(double)field(box,"minZ"),
            (double)field(box,"maxX"),(double)field(box,"maxY"),(double)field(box,"maxZ")};
    }

    static void run(Object nativeLevel) throws Exception {
        level=nativeLevel; registries=call(level,"registryAccess");
        biomeRegistry=registry("BIOME"); entityRegistry=constant("core.registries.BuiltInRegistries","ENTITY_TYPE");
        Map<String,Object> result=map("minecraft","26.1","protocol",775,"schema",1,
            "native_entry","NaturalSpawner.spawnMobsForChunkGeneration","gameplay_ticks",0,
            "entity_entropy","Explicit independent LegacyRandomSource seeds replace ONLY Entity constructor RandomSource.create; native UUID and all subsequent draws remain native.");
        @SuppressWarnings("unchecked") Map<String,Object> nativeCatalog=(Map<String,Object>)catalog();
        result.put("catalog",nativeCatalog);
        if(config.get("mode").equals("inputs")) result.put("world_inputs",GenerationSpawnInputs.capture());
        if(config.get("mode").equals("handoff")) {
            result.put("handoff_catalog",GenerationSpawnHandoff.catalog((List<?>)nativeCatalog.get("types")));
            for(String t:List.of("sheep","pig","cow","chicken","rabbit","mooshroom","horse","donkey","llama","polar_bear","parrot","turtle","wolf","fox","panda","frog","camel","armadillo","goat")) {
                for(int sample=0;sample<6;sample++) {
                    String biome=List.of("plains","snowy_plains","savanna","jungle","taiga","swamp").get(sample);
                    Map<String,Object> row=input("handoff-"+t+"-"+sample,biome,4096,0,0,t);
                    row.put("probability",0.12f); row.put("min",4); row.put("max",4);
                    row.put("environment_seed",Integer.toString(sample*17));
                    row.put("entity_seed",Long.toString(sample%2==0?Long.MIN_VALUE:1000L));
                    row.put("ground",t.equals("mooshroom")?"mycelium":t.equals("camel")||t.equals("turtle")?"sand":"grass_block");
                    if(t.equals("turtle")) row.put("ground_y",63);
                    if(t.equals("camel")) row.put("game_time","10000");
                    sample(row);
                }
            }
        }
        if(config.get("mode").equals("assets")) {
            result.put("blocks",blockMetadata((List<?>)nativeCatalog.get("types")));
            result.put("constructors",templates((List<?>)nativeCatalog.get("types")));
        }
        if(config.get("mode").equals("callbacks") || config.get("mode").equals("boundaries")) {
            result.put("constructors",templates((List<?>)nativeCatalog.get("types")));
            Map<String,Object> intervals=new TreeMap<>();
            for(String[] item:new String[][]{{"goat_jump","goat.GoatAi","TIME_BETWEEN_LONG_JUMPS"},
                {"goat_ram","goat.GoatAi","TIME_BETWEEN_RAMS"},{"frog_jump","frog.FrogAi","TIME_BETWEEN_LONG_JUMPS"}}) {
                Object provider=constant("world.entity.animal."+item[1],item[2]);
                intervals.put(item[0],List.of(call(provider,"minInclusive"),call(provider,"maxInclusive")));
            }
            result.put("memory_intervals",intervals);
            for(String t:List.of("camel","armadillo","goat","frog")) {
                String b=t.equals("camel")?"desert":t.equals("armadillo")?"savanna":t.equals("goat")?"stony_peaks":"swamp";
                int seeds=config.get("mode").equals("callbacks")?32:0;
                for(int seed=0;seed<seeds;seed++) {
                    Map<String,Object> row=input("callback-rng-"+t+"-"+seed,b,4096,0,0,t);
                    row.put("probability",0.12f); row.put("min",8); row.put("max",8);
                    row.put("environment_seed",Integer.toString(seed));
                    row.put("entity_seed",Long.toString(new long[]{0,-1,Long.MIN_VALUE,Long.MAX_VALUE}[seed%4]));
                    if(t.equals("camel")) row.put("ground","sand");
                    sample(row);
                }
                for(String rejection:List.of("collision","obstructed")) {
                    Map<String,Object> row=input("callback-reject-"+t+"-"+rejection,b,4096,-1,2,t);
                    if(t.equals("camel")) row.put("ground","sand");
                    row.put(rejection,true); sample(row);
                }
            }
            for(String t:List.of("camel","goat")) {
                Map<String,Object> row=input("callback-mob-rule-"+t,"desert",4096,0,0,t);
                row.put("ground",t.equals("camel")?"sand":"stone"); row.put("brightness",9); sample(row);
            }
        }
        if(config.get("mode").equals("suite")) {
            for(String biome:List.of("plains","forest","desert","snowy_plains","savanna","taiga","jungle","mushroom_fields","ocean"))
                for(long seed:new long[]{0,1,17,42,1234,4096,4097,4608,846692123413862008L}) {
                    Map<String,Object> row=input(biome+"-"+seed,biome,seed,0,0,null);
                    if(biome.equals("mushroom_fields")) row.put("ground","mycelium");
                    if(biome.equals("desert")) row.put("ground","sand");
                    sample(row);
                }
            for(String type:List.of("sheep","pig","cow","chicken","rabbit","mooshroom","horse","donkey","llama","polar_bear","parrot","turtle","goat","armadillo","wolf","fox","panda","frog","camel","ocelot")) {
                Map<String,Object> row=input("forced-"+type,"plains",4096,0,0,type);
                row.put("ground",type.equals("mooshroom")?"mycelium":type.equals("turtle")||type.equals("camel")?"sand":"grass_block");
                sample(row);
            }
            for(String terrain:List.of("flat","steps","water","snow","fence","low_ceiling","lateral_collision","void")) {
                Map<String,Object> row=input("terrain-"+terrain,"plains",4096,-1,2,"sheep"); row.put("terrain",terrain); sample(row);
            }
            for(int light:new int[]{0,8,9,15}) {
                Map<String,Object> row=input("light-"+light,"plains",4096,0,0,"cow"); row.put("brightness",light); sample(row);
            }
            for(String rejection:List.of("collision","obstructed","border")) {
                Map<String,Object> row=input("reject-"+rejection,"plains",4096,0,0,"pig");
                if(rejection.equals("border")) row.put("border_size",1.0); else row.put(rejection,true);
                sample(row);
            }
            sample(input("empty-table","plains",4096,0,0,"empty"));
            for(String b:List.of("snowy_plains","savanna","desert","jungle","taiga"))
                for(String t:List.of("sheep","pig","cow","chicken","rabbit")) {
                    Map<String,Object> row=input("variant-"+b+"-"+t,b,4097,-3,-5,t);
                    row.put("environment_seed","42"); row.put("entity_seed","-9223372036854775808");
                    sample(row);
                }
            for(int y:new int[]{-64,65,66,67,318,319}) {
                Map<String,Object> row=input("height-"+y,"beach",4096,0,0,"turtle");
                row.put("ground_y",y); row.put("ground","sand"); sample(row);
            }
            for(String b:List.of("snowy_plains","savanna","desert","jungle","taiga"))
                for(String t:List.of("goat","armadillo","wolf","fox","panda","frog","camel")) {
                    Map<String,Object> row=input("callback-"+b+"-"+t,b,4096,0,0,t);
                    row.put("environment_seed","42"); row.put("entity_seed","-9223372036854775808");
                    row.put("min",3); row.put("max",5);
                    if(t.equals("camel")) row.put("ground","sand");
                    sample(row);
                }
            for(long time:new long[]{0,52,53,54,10000,Long.MIN_VALUE,Long.MAX_VALUE}) {
                Map<String,Object> row=input("camel-clock-"+time,"desert",4096,0,0,"camel");
                row.put("ground","sand"); row.put("game_time",Long.toString(time)); sample(row);
            }
        }
        if(config.get("mode").equals("entry")) {
            Object cache=call(level,"getChunkSource");
            Object status=constant("world.level.chunk.status.ChunkStatus","SPAWN");
            long seed=Long.parseLong((String)config.get("seed"));
            List<Object> requests=new ArrayList<>();
            for(int z=-3;z<=3 && requests.size()<8;z++) for(int x=-3;x<=3 && requests.size()<8;x++) {
                Object rng=make("world.level.levelgen.WorldgenRandom",make("world.level.levelgen.LegacyRandomSource",0L));
                call(rng,"setDecorationSeed",seed,x<<4,z<<4);
                if((float)call(rng,"nextFloat")>=0.1f && !(x==0 && z==0)) continue;
                requests.add(List.of(x,z));
                call(cache,"getChunk",x,z,status,true);
            }
            result.put("native_requests",requests);
        }
        result.put("cases",rows); result.put("instrumented",List.copyOf(instrumented));
        Files.writeString(output.resolve("generation-spawn.json"),(String)call(gson,"toJson",result)+"\n",StandardOpenOption.CREATE_NEW);
        System.out.println("GENERATION_SPAWN_CAPTURED="+rows.size());
        Runtime.getRuntime().halt(0);
    }
}
