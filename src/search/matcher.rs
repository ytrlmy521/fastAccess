use crate::model::RecentItem;
use nucleo_matcher::{
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    Config, Matcher,
};

#[derive(Clone, Copy)]
struct Candidate<'a> {
    index: usize,
    text: &'a str,
}

impl AsRef<str> for Candidate<'_> {
    fn as_ref(&self) -> &str {
        self.text
    }
}

pub fn search(items: &[RecentItem], query: &str, limit: usize) -> Vec<RecentItem> {
    if limit == 0 {
        return Vec::new();
    }

    let query = query.trim();
    if query.is_empty() {
        return items.iter().take(limit).cloned().collect();
    }

    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let candidates = items.iter().enumerate().map(|(index, item)| Candidate {
        index,
        text: &item.search_text,
    });

    pattern
        .match_list(candidates, &mut matcher)
        .into_iter()
        .take(limit)
        .map(|(candidate, _score)| items[candidate.index].clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemKind, RecentItem};
    use std::path::PathBuf;

    fn item(path: &str, observed_at_ms: u64) -> RecentItem {
        RecentItem::new(PathBuf::from(path), observed_at_ms, ItemKind::File)
    }

    #[test]
    fn empty_query_keeps_recency_order() {
        let items = vec![item("new.txt", 2), item("old.txt", 1)];
        let matches = search(&items, "", 50);
        assert_eq!(matches[0].display_name, "new.txt");
    }

    #[test]
    fn fuzzy_query_finds_filename_and_path() {
        let items = vec![
            item(r"C:\Work\quarterly-report.docx", 2),
            item(r"C:\Photos\summer.jpg", 1),
        ];
        let matches = search(&items, "qtr rpt", 50);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].display_name, "quarterly-report.docx");
    }

    #[test]
    fn respects_result_limit() {
        let items = vec![item("alpha-1.txt", 3), item("alpha-2.txt", 2)];
        assert_eq!(search(&items, "alpha", 1).len(), 1);
    }
}
