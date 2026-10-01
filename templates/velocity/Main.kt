package {{package}}

import com.google.inject.Inject
import com.velocitypowered.api.event.Subscribe
import com.velocitypowered.api.event.proxy.ProxyInitializeEvent
import com.velocitypowered.api.plugin.Plugin
import org.slf4j.Logger

@Plugin(
    id = "{{plugin_id}}",
    name = "{{name}}",
    version = "0.1.0",
    description = "{{description_str}}",
    authors = ["{{author_str}}"],
)
class {{class_name}} @Inject constructor(private val logger: Logger) {

    @Subscribe
    fun onProxyInitialization(event: ProxyInitializeEvent) {
        logger.info("{{name}} enabled")
    }
}
