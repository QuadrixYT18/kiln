package {{package}};

import java.util.Map;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;

@RestController
public class HelloController {

    @GetMapping("/")
    public Map<String, String> hello() {
        return Map.of("message", "Hello from {{name}}");
    }

    @GetMapping("/health")
    public String health() {
        return "OK";
    }
}
