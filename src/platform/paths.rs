use std::path::PathBuf;

pub const APP_NAME: &str = "wintfy-rs";
pub const AUMID: &str = "com.wintfy-rs.app";

pub fn appdata_dir() -> Result<PathBuf, String> {
    env_path("APPDATA").map(|p| p.join(APP_NAME))
}

pub fn localappdata_dir() -> Result<PathBuf, String> {
    env_path("LOCALAPPDATA").map(|p| p.join(APP_NAME))
}

pub fn config_file() -> Result<PathBuf, String> {
    Ok(appdata_dir()?.join("config.toml"))
}

pub fn log_file() -> Result<PathBuf, String> {
    Ok(localappdata_dir()?.join("logs").join("wintfy-rs.log"))
}

pub fn install_dir() -> Result<PathBuf, String> {
    env_path("LOCALAPPDATA").map(|p| p.join("Programs").join(APP_NAME))
}

pub fn installed_exe() -> Result<PathBuf, String> {
    Ok(install_dir()?.join("wintfy-rs.exe"))
}

pub fn start_menu_shortcut() -> Result<PathBuf, String> {
    Ok(env_path("APPDATA")?
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("wintfy-rs.lnk"))
}

pub fn startup_shortcut() -> Result<PathBuf, String> {
    Ok(env_path("APPDATA")?
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("wintfy-rs.lnk"))
}

fn env_path(name: &str) -> Result<PathBuf, String> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is not set"))
}
