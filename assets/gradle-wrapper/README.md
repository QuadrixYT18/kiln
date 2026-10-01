# Embedded Gradle wrapper

`gradlew`, `gradlew.bat` and `gradle-wrapper.jar` are copied unmodified from the
[Gradle](https://github.com/gradle/gradle) repository (tag `v9.8.0`) and embedded into the
`kiln` binary. They are written into every new Gradle project by `kiln new`.

- License: [Apache-2.0](../../LICENSE-APACHE), © Gradle Inc.
- The wrapper jar is version-independent: the Gradle version a project uses is chosen by
  `gradle/wrapper/gradle-wrapper.properties`, which kiln generates with the **current**
  Gradle release (and its SHA-256 checksum) at the time of `kiln new`.
- To refresh the files, download them from the desired Gradle tag:

  ```sh
  for f in gradlew gradlew.bat gradle/wrapper/gradle-wrapper.jar; do
    curl -fsSLO "https://raw.githubusercontent.com/gradle/gradle/<tag>/$f"
  done
  ```
