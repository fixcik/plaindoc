//! ODF styles that affect extraction: list numbering and hidden tables.

use std::collections::HashMap;

use roxmltree::Document;

use super::ns::{self, STYLE, TABLE, TEXT};

#[derive(Default)]
pub(crate) struct Styles {
    /// `text:list-style` name -> per-level "numbered" flag (index 0 = level 1).
    lists: HashMap<String, Vec<bool>>,
    /// Table style name -> `table:display="false"`.
    tables: HashMap<String, bool>,
}

impl Styles {
    /// Collects from several documents (content.xml, styles.xml); the first definition of a name wins.
    pub(crate) fn collect(docs: &[&Document]) -> Styles {
        let mut s = Styles::default();
        for doc in docs {
            for n in doc.descendants() {
                let Some(name) = n.attribute((STYLE, "name")) else {
                    continue;
                };
                if ns::is(&n, TEXT, "list-style") {
                    let mut levels = vec![false; 10];
                    for l in n.children().filter(|c| c.is_element()) {
                        let numbered = ns::is(&l, TEXT, "list-level-style-number")
                            && l.attribute((STYLE, "num-format"))
                                .is_some_and(|f| !f.is_empty());
                        if let Some(i) = l
                            .attribute((TEXT, "level"))
                            .and_then(|v| v.parse::<usize>().ok())
                            .filter(|i| (1..=10).contains(i))
                        {
                            levels[i - 1] = numbered;
                        }
                    }
                    s.lists.entry(name.to_string()).or_insert(levels);
                } else if ns::is(&n, STYLE, "style")
                    && n.attribute((STYLE, "family")) == Some("table")
                {
                    let hidden = n.children().any(|p| {
                        ns::is(&p, STYLE, "table-properties")
                            && p.attribute((TABLE, "display")) == Some("false")
                    });
                    s.tables.entry(name.to_string()).or_insert(hidden);
                }
            }
        }
        s
    }

    /// `level` is 1-based (`text:level`); unknown style or level means unordered.
    pub(crate) fn list_ordered(&self, style: Option<&str>, level: usize) -> bool {
        style
            .and_then(|s| self.lists.get(s))
            .zip(level.checked_sub(1))
            .and_then(|(v, i)| v.get(i))
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn table_hidden(&self, style: Option<&str>) -> bool {
        style
            .and_then(|s| self.tables.get(s))
            .copied()
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_table_styles_scan_is_linear() {
        let depth = 20_000;
        let xml = format!(
            "<r xmlns:style=\"{STYLE}\">{}{}</r>",
            r#"<style:style style:name="t" style:family="table">"#.repeat(depth),
            "</style:style>".repeat(depth)
        );
        let doc = roxmltree::Document::parse_with_options(
            &xml,
            roxmltree::ParsingOptions {
                nodes_limit: u32::MAX,
                ..Default::default()
            },
        )
        .unwrap();
        let t = std::time::Instant::now();
        let s = Styles::collect(&[&doc]);
        assert!(!s.table_hidden(Some("t")));
        assert!(t.elapsed().as_millis() < 500);
    }
}
