# {{name}}

{{description}}

## Requirements

- JDK {{java_version}} (Gradle downloads a matching toolchain automatically)

## Build

```sh
./gradlew build
```

On Windows use `gradlew.bat build`.
{{#if paper}}

## Run a test server

```sh
./gradlew runServer
```

The task downloads Paper {{mc_version}} into `run/` and starts it with the plugin installed.
{{/if}}
{{#if velocity}}

## Install

Build the plugin with `./gradlew build` and copy `build/libs/{{name}}-0.1.0.jar` into the `plugins/` folder of your Velocity proxy.
{{/if}}
{{#if backend}}

## Run

```sh
./gradlew run
```
{{#if spring}}

The service listens on <http://localhost:8080>.
{{#else}}

The service listens on <http://localhost:8080> (`/` and `/health`).
{{/if}}
{{/if}}
{{#if docker}}

## Docker

```sh
docker compose up --build
```
{{/if}}

## Dependencies

Dependencies are managed in `gradle/libs.versions.toml`. Use [kiln](https://github.com/QuadrixYT18/kiln) to add and update them:

```sh
kiln add <name>
kiln outdated
kiln update
```
{{#if has_license}}

## License

Licensed under {{license_name}}.
{{/if}}
