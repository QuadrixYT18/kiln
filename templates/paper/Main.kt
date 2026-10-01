package {{package}}

import org.bukkit.plugin.java.JavaPlugin

class {{class_name}} : JavaPlugin() {

    override fun onEnable() {
        logger.info("{{name}} enabled")
    }

    override fun onDisable() {
        logger.info("{{name}} disabled")
    }
}
