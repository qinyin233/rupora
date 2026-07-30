#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeResult {
    pub content: String,
    pub conflicts: usize,
}

pub fn three_way_merge(base: &str, local: &str, external: &str) -> MergeResult {
    match diffy::merge(base, local, external) {
        Ok(content) => MergeResult {
            content,
            conflicts: 0,
        },
        Err(content) => MergeResult {
            conflicts: content.matches("<<<<<<<").count(),
            content,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_independent_line_changes() {
        let base = "one\ntwo\nthree\n";
        let result = three_way_merge(base, "ONE\ntwo\nthree\n", "one\ntwo\nTHREE\n");
        assert_eq!(result.content, "ONE\ntwo\nTHREE\n");
        assert_eq!(result.conflicts, 0);
    }

    #[test]
    fn marks_overlapping_changes() {
        let result = three_way_merge("one\ntwo\n", "one\nlocal\n", "one\ndisk\n");
        assert!(result.content.contains("<<<<<<<"));
        assert!(result.content.contains("local"));
        assert!(result.content.contains("disk"));
        assert_eq!(result.conflicts, 1);
    }

    #[test]
    fn handles_multiple_changes_inside_one_overlapping_region() {
        let base = "a\nb\nc\nd\n";
        let result = three_way_merge(base, "a\nLOCAL\nd\n", "a\nB\nC\nd\n");
        assert_eq!(result.conflicts, 1);
        assert!(result.content.contains("LOCAL"));
        assert!(result.content.contains("B\nC"));
    }

    #[test]
    fn accepts_a_single_changed_side_without_markers() {
        let result = three_way_merge("base", "base", "external");
        assert_eq!(result.content, "external");
        assert_eq!(result.conflicts, 0);
    }
}
