import java.util.*;

/** Native trail-ruins admission, including the actual overworld climate/height path. */
public class TrailAdmissionReference extends StructureRuntimeReference {
    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries(); configureFlat();
        List<Object> rows = new ArrayList<>();
        for (long seed : new long[]{0,1,42,-17}) {
            for (String biome : List.of("taiga","old_growth_birch_forest","plains")) {
                setup(seed,biome,"trail_ruins");
                for (int rx : new int[]{-1,0}) {
                    Object p = candidate("trail_ruins",rx,0);
                    int x = (int)call(p,"x"), z = (int)call(p,"z");
                    rows.add(admission("trail_ruins",biome,x,z));
                    rows.add(admission("trail_ruins",biome,x+1,z));
                }
            }
        }
        setup(846692123413862008L,"overworld","trail_ruins");
        int accepted = 0;
        for (int radius = 0; radius <= 10 && accepted < 3; radius++) {
            for (int x = -radius; x <= radius && accepted < 3; x++) {
                for (int z = -radius; z <= radius && accepted < 3; z++) {
                    if (Math.max(Math.abs(x),Math.abs(z)) != radius) continue;
                    Object p = candidate("trail_ruins",x,z);
                    Map<String,Object> row = admission("trail_ruins","overworld",(int)call(p,"x"),(int)call(p,"z"));
                    if (!((List<?>)row.get("starts")).isEmpty()) { rows.add(row); accepted++; }
                    else if (radius <= 1) rows.add(row);
                }
            }
        }
        if (accepted < 3) throw new IllegalStateException("expected three actual overworld starts");
        output("STRUCTURERUNTIMEREFERENCE",Map.of("admission",rows,"metadata",metadata(),
            "scope","Unmodified native trail_ruins set through ChunkGenerator.createStructures; fixed accepted/rejected biomes, negative regions and actual NoiseBasedChunkGenerator/RandomState height and climate. No full-world parity claim."));
        call(resources,"close");
    }
}
