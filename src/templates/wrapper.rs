//! Gradle wrapper files. The scripts and the (version-independent) wrapper jar
//! are embedded; `gradle-wrapper.properties` is generated with the Gradle
//! version that is current at creation time.

use super::OutFile;
use crate::registry::GradleRelease;

const GRADLEW: &str = include_str!("../../assets/gradle-wrapper/gradlew");
const GRADLEW_BAT: &str = include_str!("../../assets/gradle-wrapper/gradlew.bat");
const WRAPPER_JAR: &[u8] = include_bytes!("../../assets/gradle-wrapper/gradle-wrapper.jar");

pub fn properties(release: &GradleRelease) -> String {
    let mut s = String::new();
    s.push_str("distributionBase=GRADLE_USER_HOME\n");
    s.push_str("distributionPath=wrapper/dists\n");
    s.push_str(&format!(
        "distributionUrl=https\\://services.gradle.org/distributions/gradle-{}-bin.zip\n",
        release.version
    ));
    if let Some(sha) = &release.sha256
        && sha.len() == 64
        && sha.chars().all(|c| c.is_ascii_hexdigit())
    {
        s.push_str(&format!("distributionSha256Sum={}\n", sha.to_ascii_lowercase()));
    }
    s.push_str("networkTimeout=10000\n");
    s.push_str("validateDistributionUrl=true\n");
    s.push_str("zipStoreBase=GRADLE_USER_HOME\n");
    s.push_str("zipStorePath=wrapper/dists\n");
    s
}

pub fn files(release: &GradleRelease) -> Vec<OutFile> {
    vec![
        OutFile { path: "gradlew".into(), data: GRADLEW.replace("\r\n", "\n").into_bytes(), exec: true },
        OutFile {
            path: "gradlew.bat".into(),
            data: GRADLEW_BAT.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes(),
            exec: false,
        },
        OutFile { path: "gradle/wrapper/gradle-wrapper.jar".into(), data: WRAPPER_JAR.to_vec(), exec: false },
        OutFile {
            path: "gradle/wrapper/gradle-wrapper.properties".into(),
            data: properties(release).into_bytes(),
            exec: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_contain_version_and_checksum() {
        let p = properties(&GradleRelease { version: "9.8.0".into(), sha256: Some("a".repeat(64)) });
        assert!(p.contains("gradle-9.8.0-bin.zip"));
        assert!(p.contains(&format!("distributionSha256Sum={}", "a".repeat(64))));
        let p = properties(&GradleRelease { version: "9.8.0".into(), sha256: Some("garbage".into()) });
        assert!(!p.contains("Sha256"));
    }

    #[test]
    fn wrapper_scripts_have_proper_line_endings() {
        let f = files(&GradleRelease { version: "9.8.0".into(), sha256: None });
        let sh = f.iter().find(|x| x.path == "gradlew").unwrap();
        assert!(!sh.data.contains(&b'\r'));
        assert!(sh.exec);
        let bat = f.iter().find(|x| x.path == "gradlew.bat").unwrap();
        let t = String::from_utf8(bat.data.clone()).unwrap();
        assert_eq!(t.matches('\n').count(), t.matches("\r\n").count());
        assert_eq!(&f.iter().find(|x| x.path.ends_with(".jar")).unwrap().data[..2], b"PK");
    }
}
