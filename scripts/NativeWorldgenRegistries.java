import java.util.*;
import java.util.concurrent.*;

/** Load the JAR's actual datapack registries, including registration order/tags. */
public class NativeWorldgenRegistries extends TreeReference {
    static Object load() throws Exception {
        Object pack=call(type("server.packs.repository.ServerPacksSource"),"createVanillaPackSource");
        Object resources=make("server.packs.resources.MultiPackResourceManager",field("server.packs.PackType","SERVER_DATA"),List.of(pack));
        try {
            Object root=call(type("core.RegistryAccess"),"fromRegistryOfRegistries",field("core.registries.BuiltInRegistries","REGISTRY"));
            List<?> tags=(List<?>)call(type("tags.TagLoader"),"loadTagsForExistingRegistries",resources,root);
            Object lookups=call(type("tags.TagLoader"),"buildUpdatedLookups",root,tags);
            Object future=call(type("resources.RegistryDataLoader"),"load",resources,lookups,field("resources.RegistryDataLoader","WORLDGEN_REGISTRIES"),ForkJoinPool.commonPool());
            Object result=((CompletableFuture<?>)future).join();
            for(Object pending:tags) call(pending,"apply");
            return result;
        } finally {
            call(resources,"close");
        }
    }
}
