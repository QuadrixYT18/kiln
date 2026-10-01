plugins {
{{#if kotlin}}
    alias(libs.plugins.kotlin.jvm)
{{#else}}
    `java-library`
{{/if}}
{{#if paperweight}}
    alias(libs.plugins.paperweight.userdev)
{{/if}}
    alias(libs.plugins.run.paper)
}

group = "{{group}}"
version = "0.1.0"
description = "{{description_str}}"

repositories {
    mavenCentral()
    maven("https://repo.papermc.io/repository/maven-public/")
}

dependencies {
{{#if paperweight}}
    paperweight.paperDevBundle(libs.versions.paper.api)
{{#else}}
    compileOnly(libs.paper.api)
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
{{/if}}

tasks {
{{#unless kotlin}}
    withType<JavaCompile>().configureEach {
        options.encoding = "UTF-8"
        options.release.set({{java_version}})
    }
{{/unless}}
    processResources {
        val props = mapOf("version" to project.version)
        inputs.properties(props)
        filesMatching("paper-plugin.yml") {
            expand(props)
        }
    }
    runServer {
        minecraftVersion("{{mc_version}}")
    }
}
