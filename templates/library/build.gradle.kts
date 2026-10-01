plugins {
{{#if kotlin}}
    alias(libs.plugins.kotlin.jvm)
{{#else}}
    `java-library`
{{/if}}
    `maven-publish`
}

group = "{{group}}"
version = "0.1.0"
description = "{{description_str}}"

repositories {
    mavenCentral()
}

dependencies {
    testImplementation(platform(libs.junit.bom))
    testImplementation(libs.junit.jupiter)
    testRuntimeOnly(libs.junit.platform.launcher)
}

{{#if kotlin}}
kotlin {
    jvmToolchain({{java_version}})
}
{{#else}}
java {
    toolchain.languageVersion.set(JavaLanguageVersion.of({{java_version}}))
    withSourcesJar()
    withJavadocJar()
}

tasks.withType<JavaCompile>().configureEach {
    options.encoding = "UTF-8"
    options.release.set({{java_version}})
}
{{/if}}

tasks.test {
    useJUnitPlatform()
}

publishing {
    publications {
        create<MavenPublication>("maven") {
            from(components["java"])
        }
    }
}
