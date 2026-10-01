plugins {
{{#if kotlin}}
    alias(libs.plugins.kotlin.jvm)
{{#else}}
    java
{{/if}}
}

group = "{{group}}"
version = "0.1.0"

repositories {
    mavenCentral()
}

dependencies {
}

{{#if kotlin}}
kotlin {
    jvmToolchain({{java_version}})
}
{{#else}}
java {
    toolchain.languageVersion.set(JavaLanguageVersion.of({{java_version}}))
}
{{/if}}
