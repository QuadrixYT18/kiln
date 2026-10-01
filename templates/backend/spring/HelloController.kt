package {{package}}

import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.RestController

@RestController
class HelloController {

    @GetMapping("/")
    fun hello(): Map<String, String> = mapOf("message" to "Hello from {{name}}")

    @GetMapping("/health")
    fun health(): String = "OK"
}
