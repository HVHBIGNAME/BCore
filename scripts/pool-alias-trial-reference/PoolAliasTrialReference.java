import java.io.InputStream;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.function.BiConsumer;

/** Calls native alias codecs, forEachResolved, lookup and actual trial structures.
 * The probe contains no implementation of weighted choice or random seeding.
 */
public class PoolAliasTrialReference extends JigsawReference {
    static final String ALIAS = "world.level.levelgen.structure.pools.alias.";

    static Object json(String text) throws Exception {
        return call(Class.forName("com.google.gson.JsonParser"), "parseString", text);
    }

    static Object jsonTree(Object value) throws Exception {
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        return call(gson, "toJsonTree", value);
    }

    static Object jsonOps() throws Exception {
        return Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
    }

    static Object resource(String path) throws Exception {
        try (InputStream stream = PoolAliasTrialReference.class.getResourceAsStream(path)) {
            if (stream == null) throw new IllegalArgumentException(path);
            return json(new String(stream.readAllBytes(), StandardCharsets.UTF_8));
        }
    }

    static Object aliasCodec() throws Exception {
        return call(field(ALIAS + "PoolAliasBinding", "CODEC"), "listOf");
    }

    static List<?> decodeAliases(Object input) throws Exception {
        return (List<?>) call(call(aliasCodec(), "parse", jsonOps(), jsonTree(input)), "getOrThrow");
    }

    static String poolName(Object key) throws Exception {
        return call(key, "identifier").toString();
    }

    static Throwable root(Throwable error) {
        while (error instanceof InvocationTargetException || error instanceof UndeclaredThrowableException) {
            error = error.getCause();
        }
        return error;
    }

    static String errorText(Throwable error) {
        Throwable cause = root(error);
        return cause.getClass().getName() + ": " + cause.getMessage();
    }

    static Object direct(String alias, String target) {
        return Map.of("type", "minecraft:direct", "alias", alias, "target", target);
    }

    static Object weighted(Object value, int weight) {
        return Map.of("data", value, "weight", weight);
    }

    static Object random(String alias, Object... targets) {
        return Map.of("type", "minecraft:random", "alias", alias, "targets", List.of(targets));
    }

    static Object group(Object... groups) {
        return Map.of("type", "minecraft:random_group", "groups", List.of(groups));
    }

    static Map<String, Object> inputs() throws Exception {
        Map<String, Object> cases = new LinkedHashMap<>();
        cases.put("empty", List.of());
        cases.put("direct_chain", List.of(direct("alias/z", "alias/a"), direct("alias/a", "dest/last"), direct("other:self", "other:self")));
        cases.put("weighted_zero", List.of(random("alias/z", weighted("dest/zero", 0), weighted("dest/a", 1), weighted("dest/b", 3), weighted("dest/a", 5)), direct("alias/a", "dest/direct")));
        cases.put("singleton_draws", List.of(random("alias/one", weighted("dest/one", 1)), group(weighted(List.of(direct("alias/two", "dest/two")), 1)), random("alias/three", weighted("dest/three", 23))));
        cases.put("empty_group", List.of(group(weighted(List.of(), 1)), random("alias/after", weighted("dest/a", 1), weighted("dest/b", 1))));
        Object nested = group(weighted(List.of(random("alias/nested", weighted("dest/n1", 3), weighted("dest/n2", 2)), direct("alias/pair", "dest/p1")), 2), weighted(List.of(direct("alias/nested", "dest/n3"), direct("alias/pair", "dest/p2")), 5));
        cases.put("nested_groups", List.of(group(weighted(List.of(direct("alias/z", "dest/g1"), nested), 3), weighted(List.of(direct("alias/z", "dest/g2")), 1), weighted(List.of(direct("alias/never", "dest/zero")), 0)), random("alias/a", weighted("dest/a", 1), weighted("dest/b", 7))));
        cases.put("flat_threshold", List.of(random("alias/63", weighted("dest/a", 31), weighted("dest/b", 32)), random("alias/64", weighted("dest/a", 31), weighted("dest/b", 33))));
        cases.put("large_rejection", List.of(random("alias/reject", weighted("dest/a", 536870912), weighted("dest/b", 536870913)), random("alias/max", weighted("dest/a", Integer.MAX_VALUE - 1), weighted("dest/b", 1))));
        cases.put("duplicate", List.of(direct("alias/repeat", "dest/one"), direct("alias/repeat", "dest/two"), random("alias/after", weighted("dest/a", 1), weighted("dest/b", 2))));
        cases.put("duplicate_identical", List.of(direct("alias/repeat", "dest/same"), direct("minecraft:alias/repeat", "dest/same")));
        cases.put("conditional_duplicate", List.of(direct("alias/a", "dest/a"), group(weighted(List.of(direct("alias/a", "dest/b")), 1), weighted(List.of(direct("alias/b", "dest/b")), 1)), random("alias/after", weighted("dest/after", 1))));
        Object trial = resource("/data/minecraft/worldgen/structure/trial_chambers.json");
        cases.put("trial_chambers", call(call(trial, "getAsJsonObject"), "get", "pool_aliases"));
        cases.put("empty_targets", List.of(random("alias/a")));
        cases.put("zero_targets", List.of(random("alias/a", weighted("dest/a", 0))));
        cases.put("negative_weight", List.of(random("alias/a", weighted("dest/a", -1))));
        cases.put("overflow_total", List.of(random("alias/a", weighted("dest/a", Integer.MAX_VALUE), weighted("dest/b", 1))));
        cases.put("empty_groups", List.of(group()));
        cases.put("zero_groups", List.of(group(weighted(List.of(), 0))));
        cases.put("missing_weight", json("[{\"type\":\"random\",\"alias\":\"a\",\"targets\":[{\"data\":\"b\"}]}]"));
        cases.put("missing_data", json("[{\"type\":\"random\",\"alias\":\"a\",\"targets\":[{\"weight\":1}]}]"));
        cases.put("not_a_list", direct("alias/a", "dest/a"));
        cases.put("invalid_identifier", List.of(direct("Not Lowercase", "dest/a")));
        cases.put("unknown_type", json("[{\"type\":\"custom:direct\",\"alias\":\"a\",\"target\":\"b\"}]"));
        cases.put("empty_namespace_and_path", List.of(direct(":alias/empty_ns", "")));
        cases.put("fractional_weight", json("[{\"type\":\"random\",\"alias\":\"a\",\"targets\":[{\"data\":\"b\",\"weight\":1.75}]}]"));
        cases.put("oversized_weight", json("[{\"type\":\"random\",\"alias\":\"a\",\"targets\":[{\"data\":\"b\",\"weight\":4294967297}]}]"));
        cases.put("boolean_weight", json("[{\"type\":\"random\",\"alias\":\"a\",\"targets\":[{\"data\":\"b\",\"weight\":true}]}]"));
        cases.put("boolean_identifier", json("[{\"type\":\"direct\",\"alias\":true,\"target\":\"b\"}]"));
        cases.put("numeric_identifier", json("[{\"type\":\"direct\",\"alias\":17,\"target\":\"b\"}]"));
        cases.put("fractional_zero", json("[{\"type\":\"random\",\"alias\":\"a\",\"targets\":[{\"data\":\"zero\",\"weight\":-0.75},{\"data\":\"b\",\"weight\":1e0}]}]"));
        cases.put("invalid_zero_weight_branch", List.of(group(weighted(List.of(random("a")), 0), weighted(List.of(direct("a", "b")), 1))));
        return cases;
    }

    static List<Object> structureAliasFields() throws Exception {
        List<Object> cases = new ArrayList<>();
        Object codec = call(field("world.level.levelgen.structure.structures.JigsawStructure", "CODEC"), "codec");
        Object ops = call(type("resources.RegistryOps"), "create", jsonOps(), registries);
        for (String mode : List.of("absent", "null", "empty", "malformed")) {
            Object input = call(resource("/data/minecraft/worldgen/structure/trial_chambers.json"), "getAsJsonObject");
            call(input, "remove", "pool_aliases");
            if (!mode.equals("absent")) call(input, "add", "pool_aliases", json(mode.equals("null") ? "null" : mode.equals("empty") ? "[]" : "false"));
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("name", mode); row.put("input", input);
            try {
                Object structure = call(call(codec, "parse", ops, input), "getOrThrow");
                row.put("encoded", call(call(aliasCodec(), "encodeStart", jsonOps(), call(structure, "getPoolAliases")), "getOrThrow"));
            } catch (Exception error) { row.put("codec_error", errorText(error)); }
            cases.add(row);
        }
        return cases;
    }

    static Map<String, Object> resolution(List<?> bindings, long seed, List<Integer> position) throws Exception {
        Object pos = make("core.BlockPos", position.toArray());
        Object parent = call(type("util.RandomSource"), "create", seed);
        Object factory = call(parent, "forkPositional");
        Object random = call(factory, "at", pos);
        List<Object> draws = new ArrayList<>(), ordered = new ArrayList<>();
        Object traced = Proxy.newProxyInstance(PoolAliasTrialReference.class.getClassLoader(), new Class<?>[]{type("util.RandomSource")}, (p, m, a) -> {
            try {
                Object value = m.invoke(random, a);
                if (m.getName().equals("nextInt") && a != null && a.length == 1) draws.add(List.of(a[0], value));
                else throw new UnsupportedOperationException("unexpected alias RNG call " + m);
                return value;
            } catch (InvocationTargetException error) { throw error.getCause(); }
        });
        Set<String> queries = new TreeSet<>();
        queries.add("minecraft:not_an_alias");
        BiConsumer<Object, Object> consumer = (a, t) -> {
            try {
                String alias = poolName(a), target = poolName(t);
                ordered.add(List.of(alias, target));
                queries.add(alias); queries.add(target);
            } catch (Exception error) { throw new RuntimeException(error); }
        };
        for (Object binding : bindings) call(binding, "forEachResolved", traced, consumer);
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("seed", seed); result.put("pos", position);
        result.put("ordered", ordered); result.put("draws", draws);
        result.put("next_i64", call(random, "nextLong"));
        result.put("parent_next_i64", call(parent, "nextLong"));
        try {
            Object lookup = call(type(ALIAS + "PoolAliasLookup"), "create", bindings, pos, seed);
            Map<String, String> values = new TreeMap<>();
            for (String query : queries) values.put(query, poolName(call(lookup, "lookup", key("TEMPLATE_POOL", query))));
            result.put("lookup", values);
        } catch (Exception error) { result.put("lookup_error", errorText(error)); }
        return result;
    }

    static Map<String, Object> aliases() throws Exception {
        List<Object> cases = new ArrayList<>();
        for (Map.Entry<String, Object> entry : inputs().entrySet()) {
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("name", entry.getKey()); row.put("input", entry.getValue());
            List<?> bindings;
            try {
                bindings = decodeAliases(entry.getValue());
                row.put("encoded", call(call(aliasCodec(), "encodeStart", jsonOps(), bindings), "getOrThrow"));
            } catch (Exception error) {
                row.put("codec_error", errorText(error)); cases.add(row); continue;
            }
            List<String> targets = new ArrayList<>();
            for (Object binding : bindings) for (Object target : ((java.util.stream.Stream<?>) call(binding, "allTargets")).toList()) targets.add(poolName(target));
            row.put("all_targets", targets);
            List<Object> samples = new ArrayList<>();
            for (long seed : new long[]{0, 1, 42, -17, 918273, Long.MIN_VALUE, Long.MAX_VALUE}) {
                for (List<Integer> pos : List.of(List.of(0, 0, 0), List.of(0, -40, 0), List.of(0, -39, 0), List.of(-48, -22, 80), List.of(-6695392, 149, 5868656), List.of(Integer.MIN_VALUE, Integer.MAX_VALUE, Integer.MIN_VALUE))) {
                    samples.add(resolution(bindings, seed, pos));
                }
            }
            row.put("samples", samples); cases.add(row);
        }
        return Map.of("cases", cases, "structure_alias_fields", structureAliasFields(), "scope", "Native PoolAliasBinding.CODEC, allTargets, forEachResolved and PoolAliasLookup.create/lookup; actual Legacy positional factories, traced bounded draws and native parent/child RNG tails. Optional-field defaults are decoded by native JigsawStructure.CODEC.");
    }

    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries(); configureFlat();
        if (args[1].equals("aliases")) output("POOLALIASTRIALREFERENCE", aliases());
        else throw new IllegalArgumentException("trial capture not yet configured");
        call(resources, "close");
    }
}
