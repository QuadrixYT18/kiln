plugins {
    java
    alias(libs.plugins.shadow)
}

repositories {
    mavenCentral()
}

dependencies {
    implementation(libs.slf4j)
    implementation(libs.guava)
    testImplementation(libs.junit.jupiter)
}
