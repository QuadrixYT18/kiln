package {{package}};

/** Example API of {{name}}. */
public final class Greeter {

    private Greeter() {
    }

    /**
     * Returns a greeting.
     *
     * @param name who to greet
     * @return the greeting text
     */
    public static String greet(String name) {
        return "Hello, " + name + "!";
    }
}
