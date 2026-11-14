//! Port of `column.ts`: columns with tabs and clusters.

use crate::cluster::find_cluster_break;

/// The column at byte offset `to` (or the end) of `s`, counting a tab up
/// to the next multiple of `tab_size` and every cluster as one column
/// (`countColumn`).
pub fn count_column(s: &str, tab_size: usize, to: Option<usize>) -> usize {
    let to = to.unwrap_or(s.len());
    let mut n = 0;
    let mut i = 0;
    while i < to && i < s.len() {
        if s.as_bytes()[i] == b'\t' {
            n += tab_size - (n % tab_size);
            i += 1;
        } else {
            n += 1;
            i = find_cluster_break(s, i, true, true);
        }
    }
    n
}

/// The byte offset of column `col` in `s` (`findColumn`). When `s` is too
/// short to reach the column, returns its length, or `None` with `strict`.
pub fn find_column(s: &str, col: usize, tab_size: usize, strict: bool) -> Option<usize> {
    let mut i = 0;
    let mut n = 0;
    loop {
        if n >= col {
            return Some(i);
        }
        if i == s.len() {
            break;
        }
        n += if s.as_bytes()[i] == b'\t' {
            tab_size - (n % tab_size)
        } else {
            1
        };
        i = find_cluster_break(s, i, true, true);
    }
    if strict {
        None
    } else {
        Some(s.len())
    }
}
