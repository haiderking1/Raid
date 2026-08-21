use super::markdown::render;
use ratatui::text::Line;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

#[derive(Debug, Default)]
pub struct MarkdownCache {
    entries: HashMap<(u64, usize), Vec<Line<'static>>>,
}

impl MarkdownCache {
    pub fn lines(&mut self, source: &str, width: usize) -> &[Line<'static>] {
        let key = (hash_source(source), width.max(1));
        self.entries
            .entry(key)
            .or_insert_with(|| render(source, width.max(1)))
    }
}

fn hash_source(source: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::MarkdownCache;

    #[test]
    fn caches_by_source_and_width() {
        let mut cache = MarkdownCache::default();
        let first = cache.lines("**hi**", 20).as_ptr();
        let again = cache.lines("**hi**", 20).as_ptr();
        assert_eq!(first, again);

        let other_width = cache.lines("**hi**", 8);
        assert!(other_width.iter().any(|line| line.width() <= 8));
        assert_ne!(cache.entries.len(), 1);
    }
}
