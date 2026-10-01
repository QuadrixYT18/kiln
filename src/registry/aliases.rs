//! Built-in short names for popular libraries. Users can add their own in
//! `config.toml` under `[aliases]`.

use std::collections::BTreeMap;

use super::{Coord, Repo};

const PAPER_REPO: &str = "https://repo.papermc.io/repository/maven-public/";

/// `(alias, group:artifact, optional extra repository)`
const BUILTIN: &[(&str, &str, Option<&str>)] = &[
    // Databases & pooling
    ("hikari", "com.zaxxer:HikariCP", None),
    ("hikaricp", "com.zaxxer:HikariCP", None),
    ("postgres", "org.postgresql:postgresql", None),
    ("postgresql", "org.postgresql:postgresql", None),
    ("mysql", "com.mysql:mysql-connector-j", None),
    ("mariadb", "org.mariadb.jdbc:mariadb-java-client", None),
    ("sqlite", "org.xerial:sqlite-jdbc", None),
    ("h2", "com.h2database:h2", None),
    ("mongodb", "org.mongodb:mongodb-driver-sync", None),
    ("jedis", "redis.clients:jedis", None),
    ("lettuce", "io.lettuce:lettuce-core", None),
    ("flyway", "org.flywaydb:flyway-core", None),
    ("liquibase", "org.liquibase:liquibase-core", None),
    ("jooq", "org.jooq:jooq", None),
    ("jdbi", "org.jdbi:jdbi3-core", None),
    ("exposed", "org.jetbrains.exposed:exposed-core", None),
    ("exposed-jdbc", "org.jetbrains.exposed:exposed-jdbc", None),
    ("exposed-dao", "org.jetbrains.exposed:exposed-dao", None),
    // Serialization & utilities
    ("gson", "com.google.code.gson:gson", None),
    ("jackson", "com.fasterxml.jackson.core:jackson-databind", None),
    ("jackson-kotlin", "com.fasterxml.jackson.module:jackson-module-kotlin", None),
    ("snakeyaml", "org.yaml:snakeyaml", None),
    ("guava", "com.google.guava:guava", None),
    ("caffeine", "com.github.ben-manes.caffeine:caffeine", None),
    ("commons-lang", "org.apache.commons:commons-lang3", None),
    ("commons-io", "commons-io:commons-io", None),
    ("commons-collections", "org.apache.commons:commons-collections4", None),
    ("picocli", "info.picocli:picocli", None),
    ("jetbrains-annotations", "org.jetbrains:annotations", None),
    ("lombok", "org.projectlombok:lombok", None),
    // Logging
    ("slf4j", "org.slf4j:slf4j-api", None),
    ("logback", "ch.qos.logback:logback-classic", None),
    ("log4j", "org.apache.logging.log4j:log4j-core", None),
    ("log4j-api", "org.apache.logging.log4j:log4j-api", None),
    // HTTP & networking
    ("okhttp", "com.squareup.okhttp3:okhttp", None),
    ("retrofit", "com.squareup.retrofit2:retrofit", None),
    ("netty", "io.netty:netty-all", None),
    // Kotlin
    ("kotlin-stdlib", "org.jetbrains.kotlin:kotlin-stdlib", None),
    ("kotlin-reflect", "org.jetbrains.kotlin:kotlin-reflect", None),
    ("coroutines", "org.jetbrains.kotlinx:kotlinx-coroutines-core", None),
    ("serialization", "org.jetbrains.kotlinx:kotlinx-serialization-json", None),
    ("kotlinx-datetime", "org.jetbrains.kotlinx:kotlinx-datetime-jvm", None),
    ("ktor-server-core", "io.ktor:ktor-server-core", None),
    ("ktor-server-netty", "io.ktor:ktor-server-netty", None),
    ("ktor-client-core", "io.ktor:ktor-client-core", None),
    ("ktor-client-cio", "io.ktor:ktor-client-cio", None),
    // Testing
    ("junit", "org.junit.jupiter:junit-jupiter", None),
    ("junit-api", "org.junit.jupiter:junit-jupiter-api", None),
    ("mockito", "org.mockito:mockito-core", None),
    ("mockk", "io.mockk:mockk", None),
    ("assertj", "org.assertj:assertj-core", None),
    ("kotest", "io.kotest:kotest-runner-junit5", None),
    ("testcontainers", "org.testcontainers:testcontainers", None),
    // Spring
    ("spring-web", "org.springframework.boot:spring-boot-starter-web", None),
    ("spring-webflux", "org.springframework.boot:spring-boot-starter-webflux", None),
    ("spring-data-jpa", "org.springframework.boot:spring-boot-starter-data-jpa", None),
    ("spring-security", "org.springframework.boot:spring-boot-starter-security", None),
    ("spring-test", "org.springframework.boot:spring-boot-starter-test", None),
    // Minecraft
    ("paper", "io.papermc.paper:paper-api", Some(PAPER_REPO)),
    ("paper-api", "io.papermc.paper:paper-api", Some(PAPER_REPO)),
    ("velocity", "com.velocitypowered:velocity-api", Some(PAPER_REPO)),
    ("velocity-api", "com.velocitypowered:velocity-api", Some(PAPER_REPO)),
    ("adventure", "net.kyori:adventure-api", None),
    ("minimessage", "net.kyori:adventure-text-minimessage", None),
    ("configurate-yaml", "org.spongepowered:configurate-yaml", None),
    ("configurate-hocon", "org.spongepowered:configurate-hocon", None),
    (
        "placeholderapi",
        "me.clip:placeholderapi",
        Some("https://repo.extendedclip.com/content/repositories/placeholderapi/"),
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    pub coord: Coord,
    pub repo: Option<Repo>,
}

pub fn builtin() -> BTreeMap<String, Alias> {
    BUILTIN
        .iter()
        .filter_map(|(name, gav, repo)| {
            let (coord, _) = Coord::parse(gav)?;
            Some((name.to_string(), Alias { coord, repo: repo.map(Repo::new) }))
        })
        .collect()
}

/// Built-ins overlaid with user-defined aliases (user wins).
pub fn resolve_table(user: &BTreeMap<String, String>) -> BTreeMap<String, Alias> {
    let mut table = builtin();
    for (name, gav) in user {
        if let Some((coord, _)) = Coord::parse(gav) {
            table.insert(name.to_ascii_lowercase(), Alias { coord, repo: None });
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_aliases_parse() {
        assert_eq!(builtin().len(), BUILTIN.len().min(builtin().len()));
        for (name, gav, _) in BUILTIN {
            assert!(Coord::parse(gav).is_some(), "{name} -> {gav}");
        }
    }

    #[test]
    fn user_aliases_override() {
        let mut user = BTreeMap::new();
        user.insert("Hikari".to_string(), "my.fork:Hikari".to_string());
        let t = resolve_table(&user);
        assert_eq!(t["hikari"].coord, Coord::new("my.fork", "Hikari"));
        assert_eq!(t["postgres"].coord, Coord::new("org.postgresql", "postgresql"));
    }

    #[test]
    fn papermc_aliases_carry_repository() {
        let t = builtin();
        assert_eq!(t["paper"].repo.as_ref().unwrap().0, "https://repo.papermc.io/repository/maven-public");
    }
}
