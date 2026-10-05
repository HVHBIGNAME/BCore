import java.lang.reflect.Constructor;
import java.lang.reflect.InvocationTargetException;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** Bootstrapped native chunk dependency graphs, not a feature execution trace.
 * Compile with TreeReference.java on JDK 21; execute on the pinned JAR's JRE 25.
 */
public class FeatureDependencyReference extends TreeReference {
    static final String STATUS = "world.level.chunk.status.ChunkStatus";
    static final String PYRAMID = "world.level.chunk.status.ChunkPyramid";

    static String statusName(Object status) throws Exception {
        return (String) call(status, "getName");
    }

    static Map<String, Object> dependencies(Object dependencies) throws Exception {
        List<?> nativeList = (List<?>) call(dependencies, "asList");
        List<String> byRadius = new ArrayList<>();
        for (int radius = 0; radius < nativeList.size(); radius++) {
            Object status = call(dependencies, "get", radius);
            if (status != nativeList.get(radius)) {
                throw new IllegalStateException("dependency lookup differs from asList");
            }
            byRadius.add(statusName(status));
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("by_radius", byRadius);
        result.put("radius", call(dependencies, "getRadius"));
        return result;
    }

    static Object radiusQuery(Object dependencies, Object status) throws Exception {
        try {
            return call(dependencies, "getRadiusOf", status);
        } catch (InvocationTargetException e) {
            if (!(e.getCause() instanceof IllegalArgumentException)) throw e;
            return Map.of("throws", e.getCause().getClass().getName());
        }
    }

    static Object radiusOnlyTask(Object target) throws Exception {
        // The native constructor just stores these inputs. getRadiusForLayer
        // reads targetStatus and the pyramids; no holder, level or task is run.
        Constructor<?> constructor = type("server.level.ChunkGenerationTask").getDeclaredConstructor(
            type("server.level.GeneratingChunkMap"), type(STATUS),
            type("world.level.ChunkPos"), type("util.StaticCache2D"));
        constructor.setAccessible(true);
        return constructor.newInstance(null, target, make("world.level.ChunkPos", 0, 0), null);
    }

    static Map<String, Object> pyramid(String fieldName, boolean generation, List<?> statuses) throws Exception {
        Object pyramid = field(PYRAMID, fieldName);
        List<?> nativeSteps = (List<?>) call(pyramid, "steps");
        if (nativeSteps.size() != statuses.size()) {
            throw new IllegalStateException("status/step count differs");
        }
        List<Object> steps = new ArrayList<>();
        for (Object status : statuses) {
            int index = (int) call(status, "getIndex");
            Object step = call(pyramid, "getStepTo", status);
            if (step != nativeSteps.get(index) || call(step, "targetStatus") != status) {
                throw new IllegalStateException("native step lookup differs from status order");
            }
            Object task = radiusOnlyTask(status);
            Map<String, Object> layerRadii = new LinkedHashMap<>();
            for (Object layer : statuses.subList(0, index + 1)) {
                Object radius = call(task, "getRadiusForLayer", layer, generation);
                if (!radius.equals(call(step, "getAccumulatedRadiusOf", layer))) {
                    throw new IllegalStateException("task/step layer radius differs");
                }
                layerRadii.put(statusName(layer), radius);
            }
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("target_status", statusName(status));
            row.put("direct_dependencies", dependencies(call(step, "directDependencies")));
            row.put("accumulated_dependencies", dependencies(call(step, "accumulatedDependencies")));
            row.put("block_state_write_radius", call(step, "blockStateWriteRadius"));
            row.put("task_layer_radii", layerRadii);
            steps.add(row);
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("native_field", fieldName);
        result.put("steps", steps);
        return result;
    }

    static Map<String, Object> featureQueries(String fieldName) throws Exception {
        Object featureStatus = field(STATUS, "FEATURES");
        Object step = call(field(PYRAMID, fieldName), "getStepTo", featureStatus);
        Object direct = call(step, "directDependencies");
        Object accumulated = call(step, "accumulatedDependencies");
        Object center = make("world.level.ChunkPos", -2, 3);
        List<Object> neighbours = new ArrayList<>();
        int lastRadius = (int) call(accumulated, "getRadius") + 1;
        for (int offset = 0; offset <= lastRadius; offset++) {
            int x = -2 - offset, z = 3 + offset;
            int distance = (int) call(center, "getChessboardDistance", x, z);
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("chunk", List.of(x, z));
            row.put("native_chessboard_distance", distance);
            row.put("direct_dependency", dependencyAt(direct, distance));
            row.put("accumulated_dependency", dependencyAt(accumulated, distance));
            neighbours.add(row);
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("center", List.of(-2, 3));
        result.put("neighbour_samples", neighbours);
        result.put("direct_get_radius_of_features", radiusQuery(direct, featureStatus));
        result.put("accumulated_get_radius_of_features", radiusQuery(accumulated, featureStatus));
        result.put("step_get_accumulated_radius_of_features", call(step, "getAccumulatedRadiusOf", featureStatus));
        return result;
    }

    static Object dependencyAt(Object dependencies, int distance) throws Exception {
        return distance < (int) call(dependencies, "size")
            ? statusName(call(dependencies, "get", distance)) : "outside_dependency_table";
    }

    static Map<String, Object> capture() throws Exception {
        List<?> statuses = (List<?>) call(type(STATUS), "getStatusList");
        List<Object> statusRows = new ArrayList<>();
        for (Object status : statuses) {
            int index = (int) call(status, "getIndex");
            if (statuses.get(index) != status) {
                throw new IllegalStateException("native status index differs from list");
            }
            List<String> heightmaps = new ArrayList<>();
            for (Object heightmap : (Iterable<?>) call(status, "heightmapsAfter")) {
                heightmaps.add(((Enum<?>) heightmap).name());
            }
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("name", statusName(status));
            row.put("index", index);
            row.put("parent", statusName(call(status, "getParent")));
            row.put("chunk_type", ((Enum<?>) call(status, "getChunkType")).name());
            row.put("heightmaps_after", heightmaps);
            statusRows.add(row);
        }
        List<Object> decorationSteps = new ArrayList<>();
        for (Object value : type("world.level.levelgen.GenerationStep$Decoration").getEnumConstants()) {
            Enum<?> step = (Enum<?>) value;
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("index", step.ordinal());
            row.put("name", step.name());
            decorationSteps.add(row);
        }
        Map<String, Object> pyramids = new LinkedHashMap<>();
        pyramids.put("generation", pyramid("GENERATION_PYRAMID", true, statuses));
        pyramids.put("loading", pyramid("LOADING_PYRAMID", false, statuses));
        Map<String, Object> queries = new LinkedHashMap<>();
        queries.put("generation", featureQueries("GENERATION_PYRAMID"));
        queries.put("loading", featureQueries("LOADING_PYRAMID"));
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("schema_version", 1);
        result.put("native_pyramid_class", type(PYRAMID).getName());
        result.put("max_structure_distance", field(STATUS, "MAX_STRUCTURE_DISTANCE"));
        result.put("statuses", statusRows);
        result.put("pyramids", pyramids);
        result.put("feature_dependency_queries", queries);
        result.put("decoration_steps", decorationSteps);
        return result;
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("FEATURE_DEPENDENCY_REFERENCE=" + call(gson, "toJson", capture()));
    }
}
