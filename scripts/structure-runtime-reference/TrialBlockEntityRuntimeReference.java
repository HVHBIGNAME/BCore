import java.util.*;
import java.util.jar.*;

/** Native generated trial-spawner/vault full and update tags, including active state. */
public class TrialBlockEntityRuntimeReference extends StructureRuntimeReference {
    static Map<String, Object> observe(String name, Object state, Object load) throws Exception {
        Object position = make("core.BlockPos", 17, -31, -23);
        Object entity = call(call(state, "getBlock"), "newBlockEntity", position, state);
        Object problems = make("util.ProblemReporter$Collector");
        Object input = call(type("world.level.storage.TagValueInput"), "create", problems, registries, call(load, "copy"));
        call(entity, "loadWithComponents", input);
        if (!(boolean) call(problems, "isEmpty")) throw new IllegalStateException(problems.toString());
        return Map.of("name", name, "state", stateId(state), "pos", xyz(position),
            "load", Map.of("nbt", nbt64(load)),
            "full", Map.of("nbt", nbt64(call(entity, "saveWithFullMetadata", registries))),
            "update", Map.of("nbt", nbt64(call(entity, "getUpdateTag", registries))));
    }

    static void observeStates(List<Object> rows, String name, Object state, Object load) throws Exception {
        rows.add(observe(name, state, load));
        if (call(state, "getBlock") != field("world.level.block.Blocks", "TRIAL_SPAWNER")) return;
        Object active = call(state, "setValue", field("world.level.block.TrialSpawnerBlock", "STATE"),
            field("world.level.block.entity.trialspawner.TrialSpawnerState", "ACTIVE"));
        rows.add(observe(name + "/active", active, load));
        Object withData = call(load, "copy");
        call(withData, "putLong", "next_mob_spawns_at", 1234567890123L);
        Object entity = make("nbt.CompoundTag"), spawn = make("nbt.CompoundTag");
        call(entity, "putString", "id", "minecraft:pig");
        call(spawn, "put", "entity", entity);
        call(withData, "put", "spawn_data", spawn);
        rows.add(observe(name + "/active_next_spawn", active, withData));
    }

    static List<Object> loads(String jarPath) throws Exception {
        Set<String> names = new TreeSet<>(), seen = new HashSet<>();
        try (JarFile jar = new JarFile(jarPath)) {
            for (JarEntry entry : Collections.list(jar.entries())) {
                String path = entry.getName();
                if (path.startsWith("data/minecraft/structure/trial_chambers/") && path.endsWith(".nbt"))
                    names.add("minecraft:" + path.substring("data/minecraft/structure/".length(), path.length() - 4));
            }
        }
        List<Object> rows = new ArrayList<>();
        for (String name : names) {
            for (Object palette : (List<?>) member(template(name), "palettes")) {
                for (Object info : (List<?>) call(palette, "blocks")) {
                    Object state = call(info, "state"), tag = call(info, "nbt");
                    String block = call(blockRegistry, "getKey", call(state, "getBlock")).toString();
                    if (tag == null || !(block.equals("minecraft:trial_spawner") || block.equals("minecraft:vault"))) continue;
                    if (!seen.add(stateId(state) + ":" + nbt64(tag))) continue;
                    observeStates(rows, name, state, tag);
                }
            }
        }
        for (String block : List.of("TRIAL_SPAWNER", "VAULT"))
            observeStates(rows, "default/" + block, state(block), make("nbt.CompoundTag"));
        return rows;
    }

    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries();
        output("STRUCTURERUNTIMEREFERENCE", Map.of("scope",
            "Native trial-chambers template block entities: loadWithComponents, saveWithFullMetadata and state-dependent getUpdateTag. No gameplay ticks or spawner/vault activation.",
            "block_entity_loads", loads(args[0])));
        call(resources, "close");
    }
}
