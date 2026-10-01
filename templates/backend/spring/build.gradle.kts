plugins {
{{#if kotlin}}
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.kotlin.spring)
{{#else}}
    java
{{/if}}
    alias(libs.plugins.spring.boot)
    alias(libs.plugins.spring.dependency.management)
}

group = "{{group}}"
version = "0.1.0"

repositories {
    mavenCentral()
}

dependencies {
    implementation(libs.spring.boot.starter.web)
{{#if kotlin}}
    implementation(libs.jackson.module.kotlin)
    implementation(libs.kotlin.reflect)
{{/if}}

    testImplementation(libs.spring.boot.starter.test)
    testRuntimeOnly(libs.junit.platform.launcher)
}

{{#if kotlin}}
kotlin {
    jvmToolchain({{java_version}})
    compilerOptions {
        freeCompilerArgs.add("-Xjsr305=strict")
    }
}
{{#else}}
java {
    toolchain.languageVersion.set(JavaLanguageVersion.of({{java_version}}))
}
{{/if}}

tasks.withType<Test> {
    useJUnitPlatform()
}
