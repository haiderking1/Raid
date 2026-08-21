use std::path::PathBuf;

const AGENT_DIR_ENV: &str = "RAID_AGENT_DIR";

pub fn agent_dir() -> PathBuf {
    if let Ok(path) = std::env::var(AGENT_DIR_ENV) {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    home_dir().join(".raid").join("agent")
}

pub fn auth_path() -> PathBuf {
    agent_dir().join("auth.json")
}

pub fn settings_path() -> PathBuf {
    agent_dir().join("settings.json")
}

pub fn catalog_cache_path(plan_slug: &str) -> PathBuf {
    agent_dir().join(format!("catalog-{plan_slug}.json"))
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(unix)]
pub fn ensure_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    use std::fs::DirBuilder;
    use std::os::unix::fs::DirBuilderExt;

    if !path.exists() {
        DirBuilder::new().mode(0o700).recursive(true).create(path)?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn ensure_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

#[cfg(unix)]
pub fn write_private_file(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::fs::OpenOptions;

    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    std::io::Write::write_all(&mut file, contents.as_bytes())
}

#[cfg(not(unix))]
pub fn write_private_file(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }
    std::fs::write(path, contents)
}
