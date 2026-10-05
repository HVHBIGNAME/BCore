import java.util.*;

/** Exhaustive native state-domain checks for the two previously missing processors. */
public class StructureProcessorReference extends JigsawSupport {
    static Map<String,Object> run(String processor,float mossiness,boolean supplied) throws Exception {
        Object operation=processor.equals("blackstone_replace")?field("world.level.levelgen.structure.templatesystem.BlackstoneReplaceProcessor","INSTANCE"):
            type("world.level.levelgen.structure.templatesystem.BlockAgeProcessor").getConstructor(float.class).newInstance(mossiness);
        Object settings=make("world.level.levelgen.structure.templatesystem.StructurePlaceSettings");
        Object random=make("world.level.levelgen.LegacyRandomSource",-9321L);
        if(supplied)call(settings,"setRandom",random);
        Object origin=make("core.BlockPos",-18,-47,-20),reference=make("core.BlockPos",7,2,-9);
        Object nbt=make("nbt.CompoundTag");call(nbt,"putLong","kept",Long.MAX_VALUE-42);
        List<Integer> output=new ArrayList<>();
        Object registry=field("world.level.block.Block","BLOCK_STATE_REGISTRY");
        int count=(int)call(registry,"size");
        for(int id=0;id<count;id++) {
            Object state=call(registry,"byId",id);
            Object local=make("core.BlockPos",id%37,id%91,(id/37)%41);
            Object pos=call(local,"offset",origin);
            Object original=make("world.level.levelgen.structure.templatesystem.StructureTemplate$StructureBlockInfo",local,state,nbt);
            Object current=make("world.level.levelgen.structure.templatesystem.StructureTemplate$StructureBlockInfo",pos,state,nbt);
            Object result=call(operation,"processBlock",null,origin,reference,original,current,settings);
            if(!call(result,"pos").equals(pos)||!call(result,"nbt").equals(nbt))throw new IllegalStateException("processor discarded NBT/position "+id);
            output.add(stateId(call(result,"state")));
        }
        return Map.of("processor",processor,"mossiness",mossiness,"supplied_random",supplied,"seed",-9321,
            "states",output,"next_i64",call(random,"nextLong"),"kept",Map.of("nbt",nbt64(nbt)));
    }
    public static void main(String[] args) throws Exception {
        bootstrap(); List<Object> cases=new ArrayList<>();
        cases.add(run("blackstone_replace",0,true));
        for(float mossiness:new float[]{-0.25f,0,0.5f,1,1.25f})for(boolean supplied:new boolean[]{false,true})cases.add(run("block_age",mossiness,supplied));
        List<Object> directions=new ArrayList<>();
        for(Object direction:(Iterable<?>)field("core.Direction$Plane","HORIZONTAL"))directions.add(direction.toString());
        output("STRUCTUREPROCESSORREFERENCE",Map.of("cases",cases,"horizontal_order",directions,
            "scope","Actual native processBlock over every registered state. Both positional LegacyRandomSource and caller-supplied settings RNG; unbounded native FLOAT mossiness codec; preserved positions/NBT and final RNG."));
        call(resources,"close");
    }
}
