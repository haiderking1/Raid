use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

const NARROW_NO_BREAK_SPACE: char = '\u{202f}';

pub fn normalize_tool_path(path: &str) -> String {
    let normalized = path
        .replace('\u{00a0}', " ")
        .replace('\u{2000}', " ")
        .replace('\u{2001}', " ")
        .replace('\u{2002}', " ")
        .replace('\u{2003}', " ")
        .replace('\u{2004}', " ")
        .replace('\u{2005}', " ")
        .replace('\u{2006}', " ")
        .replace('\u{2007}', " ")
        .replace('\u{2008}', " ")
        .replace('\u{2009}', " ")
        .replace('\u{200a}', " ")
        .replace('\u{202f}', " ")
        .replace('\u{205f}', " ")
        .replace('\u{3000}', " ");
    if let Some(stripped) = normalized.strip_prefix('@') {
        stripped.to_string()
    } else {
        normalized
    }
}

pub fn resolve_tool_path(cwd: &Path, path: &str) -> PathBuf {
    resolve_path(cwd, &normalize_tool_path(path))
}

pub fn resolve_read_tool_path(cwd: &Path, path: &str) -> std::io::Result<PathBuf> {
    let resolved = resolve_tool_path(cwd, path);
    for variant in read_path_variants(&resolved) {
        if variant.exists() {
            return Ok(variant);
        }
    }
    Ok(resolved)
}

fn read_path_variants(resolved: &Path) -> Vec<PathBuf> {
    let resolved_text = resolved.to_string_lossy();
    let mut variants = vec![
        resolved.to_path_buf(),
        resolved_text
            .replace(" AM.", &format!("{NARROW_NO_BREAK_SPACE}AM."))
            .replace(" PM.", &format!("{NARROW_NO_BREAK_SPACE}PM."))
            .replace(" am.", &format!("{NARROW_NO_BREAK_SPACE}am."))
            .replace(" pm.", &format!("{NARROW_NO_BREAK_SPACE}pm."))
            .into(),
        resolved_text.nfd().collect::<String>().into(),
        resolved_text.replace('\'', "\u{2019}").into(),
        resolved_text
            .nfd()
            .collect::<String>()
            .replace('\'', "\u{2019}")
            .into(),
    ];
    variants.sort();
    variants.dedup();
    variants
}

fn resolve_path(cwd: &Path, path: &str) -> PathBuf {
    let path = path.trim();
    if path == "~" {
        return home_dir();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    if path.starts_with('/') {
        return PathBuf::from(path);
    }
    cwd.join(path)
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn clean_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => output.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            Component::Normal(part) => output.push(part),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_at_prefix_and_unicode_spaces() {
        assert_eq!(normalize_tool_path("@ src/main.rs"), " src/main.rs");
        assert_eq!(normalize_tool_path("\u{00a0}file"), " file");
    }

    #[test]
    fn resolves_relative_paths_from_cwd() {
        let cwd = PathBuf::from("/tmp/project");
        assert_eq!(
            resolve_tool_path(&cwd, "src/main.rs"),
            PathBuf::from("/tmp/project/src/main.rs")
        );
    }

    #[test]
    fn expands_home_directory() {
        let cwd = PathBuf::from("/tmp");
        let resolved = resolve_tool_path(&cwd, "~/notes.txt");
        assert!(resolved.ends_with("notes.txt"));
    }
}
