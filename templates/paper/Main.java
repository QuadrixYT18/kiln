package {{package}};

import org.bukkit.plugin.java.JavaPlugin;

public final class {{class_name}} extends JavaPlugin {

    @Override
    public void onEnable() {
        getLogger().info("{{name}} enabled");
    }

    @Override
    public void onDisable() {
        getLogger().info("{{name}} disabled");
    }
}
