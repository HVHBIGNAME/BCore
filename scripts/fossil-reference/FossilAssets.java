import java.util.*;

/** Loads the sixteen fossil templates through the native manager/data fixer. */
public class FossilAssets extends JigsawSupport {
    public static void main(String[] args) throws Exception {
        bootstrap();
        Map<String,Object> templates = new TreeMap<>();
        for (String family : List.of("spine", "skull")) for (int n = 1; n <= 4; n++) {
            for (String suffix : List.of("", "_coal")) {
                String name = "minecraft:fossil/" + family + "_" + n + suffix;
                Object template = template(name);
                List<Object> palettes = new ArrayList<>();
                for (Object palette : (List<?>)member(template, "palettes")) {
                    List<Object> blocks = new ArrayList<>();
                    for (Object info : (List<?>)call(palette, "blocks")) {
                        if (call(info,"nbt") != null) throw new IllegalStateException("unexpected fossil block entity");
                        blocks.add(Map.of("pos", xyz(call(info,"pos")), "state", stateId(call(info,"state"))));
                    }
                    palettes.add(blocks);
                }
                if (!((List<?>)member(template, "entityInfoList")).isEmpty()) throw new IllegalStateException("unexpected fossil entity");
                templates.put(name, Map.of("size", xyz(call(template,"getSize")), "palettes", palettes,
                    "native_nbt", nbt64(call(template,"save",make("nbt.CompoundTag")))));
            }
        }
        output("FOSSILASSETS", Map.of("templates", templates));
        call(resources,"close");
    }
}
