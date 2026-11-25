use std::fmt::Display;

use serde::Serialize;

/// Rust <-> TS contract: the frontend reads exactly `name` and `version`.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

fn app_info_from(name: &str, version: &impl Display) -> AppInfo {
    AppInfo {
        name: name.to_owned(),
        version: version.to_string(),
    }
}

#[tauri::command]
pub fn app_info(app: tauri::AppHandle) -> AppInfo {
    // productName 은 폴더명 제약으로 마침표가 없으므로 표시 이름 상수를 쓴다
    app_info_from(crate::native::APP_DISPLAY_NAME, &app.package_info().version)
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Validation (serialization contract)
    use super::*;

    #[test]
    fn t_ipc_01_maps_name_and_version() {
        // Arrange
        let version = "0.1.0";

        // Act
        let info = app_info_from("velio.", &version);

        // Assert
        assert_eq!(
            info,
            AppInfo {
                name: "velio.".into(),
                version: "0.1.0".into(),
            }
        );
    }

    #[test]
    fn t_ipc_02_serializes_exactly_name_and_version_keys() {
        // Arrange
        let info = app_info_from("velio.", &"0.1.0");

        // Act
        let value = serde_json::to_value(&info).expect("serialize AppInfo");

        // Assert
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["name", "version"]);
        assert_eq!(value["name"], "velio.");
        assert_eq!(value["version"], "0.1.0");
    }
}
