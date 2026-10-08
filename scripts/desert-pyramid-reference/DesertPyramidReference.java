import java.lang.reflect.*;
import java.util.*;
import java.util.function.Predicate;
import java.util.jar.JarFile;

/** Executes the real 26.1 piece, afterPlace, admission and reference paths. */
public class DesertPyramidReference extends ScatteredStructureReference {
    static final String KIND = "desert_pyramid", SET = "desert_pyramids";
    static final Map<Pos,Object> brushObserved = new HashMap<>();

    static void heightPosition(int value) throws Exception {
        Field field=type("world.level.levelgen.structure.ScatteredFeaturePiece").getDeclaredField("heightPosition");
        field.setAccessible(true);field.setInt(activePiece,value);
    }

    static List<Object> pyramidCatalog(String jarPath) throws Exception {
        try (JarFile jar = new JarFile(jarPath)) {
            Object s = structure(KIND);
            List<Object> biomes = new ArrayList<>();
            for (Object h : (Iterable<?>)call(s, "biomes")) {
                Object b = call(h, "value");
                biomes.add(List.of(call(registry("BIOME"), "getId", b), call(registry("BIOME"), "getKey", b).toString()));
            }
            Map<?,?> meta = (Map<?,?>)metadata().stream().filter(r -> ((Map<?,?>)r).get("structure").equals("minecraft:" + KIND)).findFirst().orElseThrow();
            return List.of(Map.of("kind", KIND, "set", SET, "biomes", biomes,
                "structure_id", call(registry("STRUCTURE"), "getId", s), "step", meta.get("step"), "index", meta.get("index"),
                "config", jarJson(jar, "data/minecraft/worldgen/structure/" + KIND + ".json"),
                "placement", jarJson(jar, "data/minecraft/worldgen/structure_set/" + SET + ".json")));
        }
    }

    static Map<String,Object> pyramidAdmission(String biome, int x, int z) throws Exception {
        Map<String,Object> row = new LinkedHashMap<>(admission(SET, biome, x, z));
        Object s = structure(KIND), place = call(call(holder("STRUCTURE_SET", SET), "value"), "placement");
        row.put("kind", KIND);
        row.put("candidate", call(place, "isStructureChunk", structureState, x, z));
        Object potential = call(place, "getPotentialStructureChunk", worldSeed, x, z);
        row.put("potential", List.of(call(potential,"x"), call(potential,"z")));
        Object context = make("world.level.levelgen.structure.Structure$GenerationContext", registries, generator,
            call(generator,"getBiomeSource"), randomState, templateManager, worldSeed, make("world.level.ChunkPos",x,z),
            heightAccessor, (Predicate<Object>)h -> { try { return (boolean)call(call(s,"biomes"),"contains",h); } catch(Exception e) { throw new RuntimeException(e); } });
        Object hm = field("world.level.levelgen.Heightmap$Types", "WORLD_SURFACE_WG");
        List<Object> corners = new ArrayList<>();
        for(int[] d : new int[][]{{0,0},{0,21},{21,0},{21,21}}) {
            int bx=x*16+d[0], bz=z*16+d[1];
            corners.add(List.of(bx,bz,call(generator,"getBaseHeight",bx,bz,hm,heightAccessor,randomState)));
        }
        row.put("corner_first_free",corners);
        row.put("native_lowest_y",call(type("world.level.levelgen.structure.Structure"),"getLowestY",context,21,21));
        int y=(int)call(generator,"getBaseHeight",x*16+8,z*16+8,hm,heightAccessor,randomState);
        Object biomeAt=call(call(generator,"getBiomeSource"),"getNoiseBiome",(x*16+8)>>2,(y-1)>>2,(z*16+8)>>2,call(randomState,"sampler"));
        row.put("first_free",y); row.put("sea_level",call(generator,"getSeaLevel"));
        row.put("noise_biome",call(registry("BIOME"),"getId",call(biomeAt,"value")));
        Optional<?> stub=(Optional<?>)call(s,"findValidGenerationPoint",context);
        row.put("biome_admitted",stub.isPresent());
        if(stub.isPresent()) {
            Object value=stub.orElseThrow(); row.put("generation_point",xyz(call(value,"position")));
            Object start=make("world.level.levelgen.structure.StructureStart",s,make("world.level.ChunkPos",x,z),0,call(call(value,"getPiecesBuilder"),"build"));
            row.put("assembled",Map.of("nbt",nbt64(call(start,"createTag",serialContext,make("world.level.ChunkPos",x,z)))));
        }
        row.put("next_i64",call(call(context,"random"),"nextLong"));
        return row;
    }

    static List<Object> pyramidAdmissions() throws Exception {
        List<Object> rows=new ArrayList<>();
        for(long seed:new long[]{0,1,42,-17,Long.MIN_VALUE,Long.MAX_VALUE}) for(String biome:List.of("desert","plains")) {
            setup(seed,biome,SET);
            for(int rx:new int[]{-1,0}) {
                Object p=candidate(SET,rx,-1); int x=(int)call(p,"x"),z=(int)call(p,"z");
                rows.add(pyramidAdmission(biome,x,z)); rows.add(pyramidAdmission(biome,x+1,z));
            }
        }
        for(int layers:new int[]{0,1,2,3,126,127,128,129}) {
            setup(42,"desert",SET);
            Object settings=make("world.level.levelgen.flat.FlatLevelGeneratorSettings",Optional.empty(),holder("BIOME","desert"),List.of());
            if(layers>0)((List<Object>)call(settings,"getLayersInfo")).add(make("world.level.levelgen.flat.FlatLayerInfo",layers,field("world.level.block.Blocks","STONE")));
            call(settings,"updateLayers"); generator=make("world.level.levelgen.FlatLevelSource",settings);
            Object p=candidate(SET,-1,0);
            Map<String,Object> row=pyramidAdmission("desert",(int)call(p,"x"),(int)call(p,"z"));
            row.put("flat_layers",layers); rows.add(row);
        }
        setup(846692123413862008L,"overworld",SET);
        int accepted=0,rejected=0;
        for(int radius=0;radius<=32 && accepted<2;radius++) {
            for(int rx=-radius;rx<=radius && accepted<2;rx++) for(int rz=-radius;rz<=radius && accepted<2;rz++) {
                if(Math.max(Math.abs(rx),Math.abs(rz))!=radius)continue;
                Object p=candidate(SET,rx,rz);
                Map<String,Object> row=pyramidAdmission("overworld",(int)call(p,"x"),(int)call(p,"z"));
                if(!((List<?>)row.get("starts")).isEmpty()) { rows.add(row); accepted++;output("PYRAMIDADMISSION",row); }
                else if(rejected++<4)rows.add(row);
            }
        }
        if(accepted!=2)throw new IllegalStateException("two real pyramid starts required");
        return rows;
    }

    static void observeBrush() throws Exception {
        List<Pos> positions=new ArrayList<>(blockEntities.keySet());positions.sort(POS_ORDER);
        for(Pos p:positions) {
            Object entity=blockEntities.get(p);
            if(!type("world.level.block.entity.BrushableBlockEntity").isInstance(entity))continue;
            Object table=member(entity,"lootTable");
            if(table==null)continue;
            Object signature=List.of(entity,table,member(entity,"lootTableSeed"));
            if(!signature.equals(brushObserved.put(p,signature)))effects.add(List.of("loot",p.x(),p.y(),p.z(),"minecraft:brushable_block",
                call(table,"identifier").toString(),member(entity,"lootTableSeed")));
        }
    }

    static Object pyramidWorld() throws Exception {
        Object delegate=scatteredWorld();
        return Proxy.newProxyInstance(DesertPyramidReference.class.getClassLoader(),new Class<?>[]{type("world.level.WorldGenLevel")},(p,m,a)->{
            try {
                observeBrush();
                if(m.getName().equals("getRandom"))return regionRandom;
                return m.invoke(delegate,a);
            } catch(InvocationTargetException e) { throw e.getCause(); }
        });
    }

    static Map<String,Object> archaeologyState(Object piece) throws Exception {
        List<Object> candidates=new ArrayList<>();
        for(Object p:(List<?>)call(piece,"getPotentialSuspiciousSandWorldPositions"))candidates.add(xyz(p));
        return Map.of("potential",candidates,"roof",xyz(call(piece,"getRandomCollapsedRoofPos")));
    }

    static Map<String,Object> pyramidPlacement(String name,long seed,int x,int z,String mode,int shift,
        int[][] sources,String special) throws Exception {
        configureFlat();freshWorld(mode,seed);brushObserved.clear();
        Object cp=make("world.level.ChunkPos",x,z),start=nativeStart("minecraft:"+KIND,seed,x,z);
        activePiece=((List<?>)call(start,"getPieces")).get(0);
        if(shift!=0)call(activePiece,"move",shift,0,shift);
        Map<String,Object> row=new LinkedHashMap<>();
        row.put("name",name);row.put("kind",KIND);row.put("seed",seed);row.put("chunk",List.of(x,z));
        row.put("shift",List.of(shift,0,shift));row.put("terrain",mode);row.put("special",special);
        row.put("orientation",call(activePiece,"getOrientation").toString());
        if(special.equals("missing_entity"))suppressBlockEntities=true;
        if(special.equals("foundation")) {
            // Pin HPos to isolate fillColumnDown's vegetation/fluid replacements.
            heightPosition(64);
            int i=0;
            for(String material:List.of("WATER","LAVA","SEAGRASS","TALL_SEAGRASS","GLOW_LICHEN","KELP","DIRT")) {
                for(int y=-5;y>=-9;y--)overrides.put(Pos.from(call(activePiece,"getWorldPos",i,y,0)),cached(material));
                i++;
            }
        }
        if(special.equals("deny_containers")) {
            heightPosition(64);
            for(int[] p:new int[][]{{10,-11,8},{12,-11,10},{10,-11,12},{8,-11,10}})
                denied.add(Pos.from(call(activePiece,"getWorldPos",p[0],p[1],p[2])));
        }
        if(special.equals("high")) { call(activePiece,"move",0,247,0);heightPosition(311); }
        row.put("initial",Map.of("nbt",nbt64(call(start,"createTag",serialContext,cp))));
        row.put("reference_bounds",bounds(call(start,"getBoundingBox")));
        row.put("initial_archaeology",archaeologyState(activePiece));
        row.put("overrides",overrideRows());row.put("denied",denied.stream().sorted(POS_ORDER).map(p->List.of(p.x(),p.y(),p.z())).toList());
        row.put("suppress_block_entities",suppressBlockEntities);row.put("min_y",worldMinY);
        Map<?,?> meta=(Map<?,?>)metadata().stream().filter(r->((Map<?,?>)r).get("structure").equals("minecraft:"+KIND)).findFirst().orElseThrow();
        Object settings=holder("NOISE_SETTINGS","overworld");
        Object rs=call(type("world.level.levelgen.RandomState"),"create",call(settings,"value"),registry("NOISE"),seed);
        Object factory=call(rs,"getOrCreateRandomFactory",identifier("minecraft:worldgen_region_random"));
        List<Object> passes=new ArrayList<>();
        for(int passIndex=0;passIndex<sources.length;passIndex++) {
            int[] source=sources[passIndex];writeSourceX=source[0];writeSourceZ=source[1];
            if(special.equals("reload") && passIndex>0) {
                start=call(type("world.level.levelgen.structure.StructureStart"),"loadStaticStart",serialContext,call(start,"createTag",serialContext,cp),seed);
                activePiece=((List<?>)call(start,"getPieces")).get(0);
            }
            writes=new ArrayList<>();ticks=new ArrayList<>();effects=new ArrayList<>();heightQueries=new ArrayList<>();
            markChunks=new HashMap<>();observedMarks=new HashMap<>();
            Object clip=make("world.level.levelgen.structure.BoundingBox",source[0]*16,source.length>2?source[2]:-63,source[1]*16,source[0]*16+15,source.length>2?source[3]:319,source[1]*16+15);
            Object random=make("world.level.levelgen.WorldgenRandom",make("world.level.levelgen.XoroshiroRandomSource",0L));
            long decoration=(long)call(random,"setDecorationSeed",seed,source[0]*16,source[1]*16);
            call(random,"setFeatureSeed",decoration,meta.get("index"),meta.get("step"));
            regionRandom=call(factory,"at",make("core.BlockPos",source[0]*16,0,source[1]*16));
            int advance=special.equals("advanced_region")?37:0;
            for(int i=0;i<advance;i++)call(regionRandom,"nextLong");
            Map<String,Object> pass=new LinkedHashMap<>();
            pass.put("before_archaeology",archaeologyState(activePiece));pass.put("region_advance",advance);
            call(start,"placeInChunk",pyramidWorld(),null,generator,random,clip,make("world.level.ChunkPos",source[0],source[1]));
            observeBrush();observeEffects();
            pass.putAll(runtimeSnapshot());pass.put("source",List.of(source[0],source[1]));pass.put("clip",bounds(clip));
            pass.put("effects",new ArrayList<>(effects));pass.put("height_queries",new ArrayList<>(heightQueries));
            pass.put("decoration_seed",decoration);pass.put("next_i64",call(random,"nextLong"));pass.put("region_next_i64",call(regionRandom,"nextLong"));
            Object tag=call(start,"createTag",serialContext,cp);
            pass.put("after",Map.of("nbt",nbt64(tag)));pass.put("archaeology",archaeologyState(activePiece));
            pass.put("cached_reference_bounds",bounds(call(start,"getBoundingBox")));
            Object reloaded=call(type("world.level.levelgen.structure.StructureStart"),"loadStaticStart",serialContext,tag,seed);
            pass.put("reloaded_reference_bounds",bounds(call(reloaded,"getBoundingBox")));
            pass.put("reloaded_archaeology",archaeologyState(((List<?>)call(reloaded,"getPieces")).get(0)));
            passes.add(pass);
        }
        row.put("passes",passes);return row;
    }

    static List<Object> pyramidPlacements() throws Exception {
        List<Object> rows=new ArrayList<>();
        for(long seed:new long[]{0,1,2,3,4,5,6,7,8,9,42,-17})
            rows.add(pyramidPlacement("orientation_"+seed,seed,-2,3,"flat",0,new int[][]{{-2,3},{-1,3},{-2,4},{-1,4},{-2,3}},""));
        for(String mode:List.of("slope","water","void"))
            rows.add(pyramidPlacement(mode,42,-1,-1,mode,13,new int[][]{{-1,-1},{0,-1},{1,-1},{-1,0},{0,0},{1,0},{-1,1},{0,1},{1,1},{0,0}},""));
        rows.add(pyramidPlacement("reverse",42,-1,-1,"slope",13,new int[][]{{1,1},{0,1},{-1,1},{1,0},{0,0},{-1,0},{1,-1},{0,-1},{-1,-1},{0,0}},""));
        for(String special:List.of("missing_entity","deny_containers","foundation","advanced_region","reload","high"))
            rows.add(pyramidPlacement(special,-17,-2,3,"flat",0,new int[][]{{-2,3},{-1,3},{-2,4},{-1,4},{-2,3}},special));
        rows.add(pyramidPlacement("vertical_clip",0,0,0,"flat",0,new int[][]{{0,0,65,319},{1,0,65,319},{0,1,65,319},{1,1,65,319}},""));
        rows.add(pyramidPlacement("after_place_without_piece",42,0,0,"flat",0,new int[][]{{0,0,-63,0},{0,0},{0,0,-63,61}},""));
        return rows;
    }

    static List<Object> pyramidReferences() throws Exception {
        List<Object> rows=new ArrayList<>();
        for(int shift:new int[]{0,13}) {
            configureFlat();freshWorld("flat",42);
            Object cp=make("world.level.ChunkPos",-1,-1),s=structure(KIND),start=nativeStart("minecraft:"+KIND,42,-1,-1);
            if(shift!=0)call(((List<?>)call(start,"getPieces")).get(0),"move",shift,0,shift);
            call(markChunk(-1,-1),"setStartForStructure",s,start);
            structureManager=make("world.level.StructureManager",referenceWorld(),make("world.level.levelgen.WorldOptions",42L,true,false),null);
            List<Object> targets=new ArrayList<>();
            for(int x=-3;x<=3;x++)for(int z=-3;z<=3;z++) {
                Object chunk=markChunk(x,z);call(generator,"createReferences",referenceWorld(),structureManager,chunk);
                Object refs=((Map<?,?>)call(chunk,"getAllReferences")).get(s);List<Object> found=new ArrayList<>();
                if(refs!=null)for(Object packed:(Iterable<?>)refs) {
                    Object p=call(type("world.level.ChunkPos"),"unpack",packed);found.add(List.of(call(p,"x"),call(p,"z")));
                }
                targets.add(Map.of("chunk",List.of(x,z),"sources",found));
            }
            rows.add(Map.of("start",Map.of("nbt",nbt64(call(start,"createTag",serialContext,cp))),"reference_bounds",bounds(call(start,"getBoundingBox")),"targets",targets));
        }
        return rows;
    }

    public static void main(String[] args) throws Exception {
        bootstrap();includeBuiltInRegistries();configureFlat();
        String section=args.length>1?args[1]:"all";
        Map<String,Object> result=new LinkedHashMap<>();
        result.put("scope","Native 26.1 DesertPyramidPiece.postProcess + DesertPyramidStructure.afterPlace through StructureStart.placeInChunk, real admission/createReferences, scripted solid/fluid/void worlds. Separate caller feature RNG and native source-region RNG, exact ordered writes, height reads, typed full/update block entities, transient archaeology state, chest flags, native save/reload and RNG continuations. No gameplay ticks or FULL lifecycle substitution.");
        result.put("catalog",pyramidCatalog(args[0]));
        if(!section.equals("placement"))result.put("admission",pyramidAdmissions());
        if(!section.equals("admission")) {result.put("placements",pyramidPlacements());result.put("references",pyramidReferences());}
        output("DESERTPYRAMIDREFERENCE",result);call(resources,"close");
    }
}
