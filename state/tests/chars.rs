//! Port of `@marijn/find-cluster-break` 1.0.4 `test/test-cluster.js`, with
//! cases for `char.ts` and `column.ts`, which upstream does not test
//! directly.

use editor_state::{
    code_point_at, code_point_size, count_column, find_cluster_break, find_column, from_code_point,
};

// Upstream's helper: `|` marks the expected breaks. Positions are UTF-8
// byte offsets here, as `indexOf` gives UTF-16 offsets upstream.
fn check(spec: &str) {
    let text = spec.replace('|', "");
    let breaks: Vec<usize> = spec
        .match_indices('|')
        .enumerate()
        .map(|(i, (at, _))| at - i)
        .collect();
    let mut found = Vec::new();
    let mut i = 0;
    loop {
        let next = find_cluster_break(&text, i, true, true);
        if next == text.len() {
            break;
        }
        i = next;
        found.push(i);
    }
    assert_eq!(found, breaks, "forward breaks in {spec:?}");
    // Not upstream: walking backward finds the same breaks.
    let mut back = Vec::new();
    let mut i = text.len();
    loop {
        let prev = find_cluster_break(&text, i, false, true);
        if prev == 0 {
            break;
        }
        i = prev;
        back.push(i);
    }
    back.reverse();
    assert_eq!(back, breaks, "backward breaks in {spec:?}");
}

#[test]
fn breaks_plain_text() {
    check("a|b|c|d");
}

#[test]
fn keeps_combining_marks_with_their_base() {
    check("a|é̠|ő|x");
}

#[test]
fn breaks_between_astral_characters() {
    check("😎|🙉");
}

#[test]
fn keeps_zwj_sequences_and_modifiers_together() {
    check("👨‍🎤|💪🏽|👩‍👩‍👧‍👦|❤");
}

#[test]
fn pairs_regional_indicators() {
    check("🇩🇪|🇫🇷|🇪🇸|x|🇮🇹");
}

#[test]
fn can_exclude_extending_characters() {
    let s = "e\u{301}x";
    assert_eq!(find_cluster_break(s, 0, true, true), 3);
    assert_eq!(find_cluster_break(s, 0, true, false), 1);
    assert_eq!(find_cluster_break(s, 3, false, false), 1);
    assert_eq!(find_cluster_break(s, 0, false, true), 0);
    assert_eq!(find_cluster_break(s, s.len(), true, true), s.len());
}

#[test]
#[should_panic(expected = "not a character boundary")]
fn rejects_positions_inside_characters() {
    find_cluster_break("é", 1, true, true);
}

#[test]
fn reads_code_points() {
    assert_eq!(code_point_at("a😎", 1), 0x1F60E);
    assert_eq!(from_code_point(0x1F60E), "😎");
    assert_eq!(from_code_point(0xD800), "\u{FFFD}");
    assert_eq!(code_point_size('a' as u32), 1);
    assert_eq!(code_point_size('é' as u32), 2);
    assert_eq!(code_point_size('中' as u32), 3);
    assert_eq!(code_point_size(0x1F60E), 4);
}

#[test]
fn counts_columns() {
    assert_eq!(count_column("\tab", 4, None), 6);
    assert_eq!(count_column("a\tb", 4, None), 5);
    assert_eq!(count_column("a\tb", 4, Some(1)), 1);
    assert_eq!(count_column("e\u{301}x", 4, None), 2);
    assert_eq!(count_column("😎x", 2, Some(4)), 1);
}

#[test]
fn finds_columns() {
    assert_eq!(find_column("\tab", 5, 4, false), Some(2));
    assert_eq!(find_column("\tab", 2, 4, false), Some(1));
    assert_eq!(find_column("e\u{301}x", 1, 4, false), Some(3));
    assert_eq!(find_column("ab", 5, 4, false), Some(2));
    assert_eq!(find_column("ab", 5, 4, true), None);
}
