import java.util.*;

/** Native overworld climate rows and datapack biome registry from the target JAR. */
public class WorldgenDataReference extends TreeReference {
    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"),"tryDetectVersion"); call(type("server.Bootstrap"),"bootStrap");
        Object registries=NativeWorldgenRegistries.load();
        Object biomes=call(registries,"lookupOrThrow",field("core.registries.Registries","BIOME"));
        Object parameters=make("world.level.biome.MultiNoiseBiomeSourceParameterList",field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset","OVERWORLD"),biomes);
        Object ops=Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        List<Object> rows=new ArrayList<>();
        for(Object pair:(List<?>)call(call(parameters,"parameters"),"values")) {
            Object point=call(pair,"getFirst"), holder=call(pair,"getSecond");
            Object encoded=call(call(field("world.level.biome.Climate$ParameterPoint","CODEC"),"encodeStart",ops,point),"getOrThrow");
            rows.add(Map.of("parameters",encoded,"biome",call(call(holder,"key"),"identifier").toString()));
        }
        List<String> ids=new ArrayList<>();
        for(Object biome:(Iterable<?>)biomes) {
            if((int)call(biomes,"getId",biome)!=ids.size()) throw new IllegalStateException("non-contiguous biome ids");
            ids.add(call(biomes,"getKey",biome).toString());
        }
        Object gson=Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("WORLDGEN_DATA_REFERENCE="+call(gson,"toJson",Map.of("parameters",Map.of("biomes",rows),"biome_registry",ids,"samples",List.of())));
    }
}
