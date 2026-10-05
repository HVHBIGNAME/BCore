import java.util.*;

/** Unmodified trial-chambers set through actual ChunkGenerator admission. */
public class TrialAdmissionReference extends StructureRuntimeReference {
    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries(); configureFlat();
        List<Object> rows = new ArrayList<>();
        for (long seed : new long[]{0,1,42,-17}) {
            for (String biome : List.of("plains","lush_caves","deep_dark")) {
                setup(seed,biome,"trial_chambers");
                for (int rx : new int[]{-1,0}) {
                    Object p = candidate("trial_chambers",rx,0);
                    int x = (int)call(p,"x"), z = (int)call(p,"z");
                    rows.add(admission("trial_chambers",biome,x,z));
                    rows.add(admission("trial_chambers",biome,x+1,z));
                }
            }
        }
        setup(846692123413862008L,"overworld","trial_chambers");
        int accepted = 0;
        for (int radius = 0; radius <= 10 && accepted < 3; radius++) {
            for (int x = -radius; x <= radius && accepted < 3; x++) {
                for (int z = -radius; z <= radius && accepted < 3; z++) {
                    if (Math.max(Math.abs(x),Math.abs(z)) != radius) continue;
                    Object p = candidate("trial_chambers",x,z);
                    Map<String,Object> row = admission("trial_chambers","overworld",(int)call(p,"x"),(int)call(p,"z"));
                    if (!((List<?>)row.get("starts")).isEmpty()) { rows.add(row); accepted++; }
                    else if (radius <= 1) rows.add(row);
                }
            }
        }
        if (accepted < 3) throw new IllegalStateException("expected three actual overworld starts");
        output("STRUCTURERUNTIMEREFERENCE",Map.of("admission",rows,"metadata",metadata(),
            "scope","Native trial_chambers set through ChunkGenerator.createStructures: fixed biomes, negative regions, off-candidate chunks and real NoiseBasedChunkGenerator climate. Retained starts are saved after admission; no gameplay ticks."));
        call(resources,"close");
    }
}
