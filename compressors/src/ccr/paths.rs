use std::path::PathBuf;

/// Resolve the default data directory for the CCR database.
pub fn dirs_or_default() -> PathBuf {
    if let Some(data_dir) = std::env::var_os("RETOKEN_DATA_DIR") {
        return PathBuf::from(data_dir);
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("LOCALAPPDATA") {
            let mut p = PathBuf::from(appdata);
            p.push("ReToken");
            return p;
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let mut p = PathBuf::from(home);
            p.push(".retoken");
            return p;
        }
    }
    PathBuf::from(".")
}
