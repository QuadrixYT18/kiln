package {{package}};

import com.google.inject.Inject;
import com.velocitypowered.api.event.Subscribe;
import com.velocitypowered.api.event.proxy.ProxyInitializeEvent;
import com.velocitypowered.api.plugin.Plugin;
import org.slf4j.Logger;

@Plugin(
        id = "{{plugin_id}}",
        name = "{{name}}",
        version = "0.1.0",
        description = "{{description_str}}",
        authors = {"{{author_str}}"}
)
public final class {{class_name}} {

    private final Logger logger;

    @Inject
    public {{class_name}}(Logger logger) {
        this.logger = logger;
    }

    @Subscribe
    public void onProxyInitialization(ProxyInitializeEvent event) {
        logger.info("{{name}} enabled");
    }
}
