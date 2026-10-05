/** Initialize the real item components before the shared trial-asset extractor.
 * No template, block-entity, codec, or random implementation is replaced.
 */
public class PoolAliasTrialAssetsBootstrap extends JigsawReference {
    public static void main(String[] args) throws Exception {
        bootstrap();
        includeBuiltInRegistries();
        call(resources, "close");
        // JigsawAssets reloads worldgen registries. Item holders are global and
        // now have their native components, required by VaultConfig's default.
        JigsawAssets.main(args);
    }
}
