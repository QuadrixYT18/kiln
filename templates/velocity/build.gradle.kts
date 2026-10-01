plugins {
{{#if kotlin}}
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.kotlin.kapt)
{{#else}}
    `java-library`
{{/if}}
}

group = "{{group}}"
version = "0.1.0"
description = "{{description_str}}"

repositories {
    mavenCentral()
    maven("https://repo.papermc.io/repository/maven-public/")
}

dependencies {
    compileOnly(libs.velocity.api)
{{#if kotlin}}
    kapt(libs.velocity.api)
{{#else}}
    annotationProcessor(libs.velocity.api)
{{/if}}
}

{{#if kotlin}}
kotlin {
    jvmToolchain({{java_version}})
}
{{#else}}
java {
    toolchain.languageVersion.set(JavaLanguageVersion.of({{java_version}}))
}

tasks.withType<JavaCompile>().configureEach {
    options.encoding = "UTF-8"
    options.release.set({{java_version}})
}
{{/if}}
