package {{package}}

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class GreeterTest {

    @Test
    fun greetsByName() {
        assertEquals("Hello, kiln!", Greeter.greet("kiln"))
    }
}
