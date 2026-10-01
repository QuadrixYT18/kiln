plugins {
    java
    id("com.gradleup.shadow") version "8.3.0" // fat jar
}

group = "com.example"
version = "1.0.0"

val guavaVersion = "32.0.0-jre"

repositories {
    mavenCentral()
}

dependencies {
    // logging
    implementation("org.slf4j:slf4j-api:2.0.9")
    implementation("com.google.guava:guava:$guavaVersion")

    testImplementation("org.junit.jupiter:junit-jupiter:5.10.0")
}

tasks.test {
    useJUnitPlatform()
}
