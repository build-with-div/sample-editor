//! Port of `@codemirror/state` 6.7.6 `test/test-text.ts`.

mod common;

use common::{coin, r};
use editor_state::{Text, TextIterator};

fn depth(node: &Text) -> usize {
    match node.children() {
        None => 1,
        Some(children) => 1 + children.iter().map(depth).max().unwrap_or(0),
    }
}

fn line() -> String {
    "1234567890".repeat(10)
}

fn lines() -> Vec<String> {
    vec![line(); 200]
}

fn text0() -> String {
    lines().join("\n")
}

fn doc0() -> Text {
    Text::of(&lines()).unwrap()
}

fn of(lines: &[&str]) -> Text {
    Text::of(lines).unwrap()
}

#[test]
fn handles_basic_replacement() {
    let doc = of(&["one", "two", "three"]);
    assert_eq!(
        doc.replace(2, 5, &of(&["foo", "bar"])).to_string(),
        "onfoo\nbarwo\nthree"
    );
}

#[test]
fn can_append_documents() {
    assert_eq!(
        of(&["one", "two", "three"])
            .append(&of(&["!", "ok"]))
            .to_string(),
        "one\ntwo\nthree!\nok"
    );
}

#[test]
fn preserves_length() {
    assert_eq!(doc0().len(), text0().len());
}

#[test]
fn creates_a_balanced_tree_when_loading_a_document() {
    let doc = Text::of(&vec![line(); 2000]).unwrap();
    assert!(depth(&doc) <= 2);
}

#[test]
fn rebalances_on_insert() {
    let mut doc = doc0();
    let insert = "abc".repeat(200);
    let at = doc.len() / 2;
    for _ in 0..10 {
        doc = doc.replace(at, at, &of(&[&insert]));
    }
    assert!(depth(&doc) <= 2);
    let text0 = text0();
    assert_eq!(
        doc.to_string(),
        format!("{}{}{}", &text0[..at], "abc".repeat(2000), &text0[at..])
    );
}

#[test]
fn collapses_on_delete() {
    let doc = doc0().replace(10, text0().len() - 10, &Text::empty());
    assert_eq!(depth(&doc), 1);
    assert_eq!(doc.len(), 20);
    assert_eq!(doc.to_string(), &line()[..20]);
}

#[test]
fn handles_deleting_at_start() {
    let mut lines = lines();
    let last = lines.len() - 1;
    lines[last] = line() + "!";
    assert_eq!(
        Text::of(&lines)
            .unwrap()
            .replace(0, 9500, &Text::empty())
            .to_string(),
        format!("{}!", &text0()[9500..])
    );
}

#[test]
fn handles_deleting_at_end() {
    let mut lines = lines();
    lines[0] = format!("?{}", line());
    assert_eq!(
        Text::of(&lines)
            .unwrap()
            .replace(9500, text0().len() + 1, &Text::empty())
            .to_string(),
        format!("?{}", &text0()[..9499])
    );
}

#[test]
fn can_handle_deleting_the_entire_document() {
    let doc = doc0();
    assert_eq!(doc.replace(0, doc.len(), &Text::empty()).to_string(), "");
}

#[test]
fn can_insert_on_node_boundaries() {
    let doc = doc0();
    let pos = doc.children().unwrap()[0].len();
    assert_eq!(
        doc.replace(pos, pos, &of(&["abc"]))
            .slice(pos, pos + 3)
            .to_string(),
        "abc"
    );
}

#[test]
fn can_build_up_a_doc_by_repeated_appending() {
    let mut doc = of(&[""]);
    let mut text = String::new();
    for i in 1..1000 {
        let add = format!("newtext{i} ");
        doc = doc.replace(doc.len(), doc.len(), &of(&[&add]));
        text += &add;
    }
    assert_eq!(doc.to_string(), text);
}

#[test]
fn properly_maintains_content_during_editing() {
    let mut str = text0();
    let mut doc = doc0();
    for _ in 0..200 {
        let ins_pos = r(doc.len());
        let ins_char = ((b'A' + r(26) as u8) as char).to_string();
        str.insert_str(ins_pos, &ins_char);
        doc = doc.replace(ins_pos, ins_pos, &of(&[&ins_char]));
        let del_from = r(doc.len());
        let del_to = doc.len().min(del_from + r(20));
        str.replace_range(del_from..del_to, "");
        doc = doc.replace(del_from, del_to, &Text::empty());
    }
    assert_eq!(doc.to_string(), str);
}

#[test]
fn returns_the_correct_strings_for_slice() {
    let text: Vec<String> = (0..1000).map(|i| format!("{i:04}")).collect();
    let doc = Text::of(&text).unwrap();
    let str = text.join("\n");
    for i in 0..400 {
        let start = if i == 0 { 0 } else { r(doc.len()) };
        let end = if i == 399 {
            doc.len()
        } else {
            start + r(doc.len() - start)
        };
        assert_eq!(doc.slice(start, end).to_string(), &str[start..end]);
    }
    // Upstream overrides every pair with this one; check it too.
    assert_eq!(doc.slice(4150, 4160).to_string(), &str[4150..4160]);
}

#[test]
fn can_be_compared() {
    let doc = doc0();
    let doc2 = Text::of(&lines()).unwrap();
    assert!(doc == doc);
    assert!(doc == doc2);
    assert!(doc2 == doc);
    assert!(doc != doc2.replace(5000, 5000, &of(&["y"])));
    assert!(doc != doc2.replace(5000, 5001, &of(&["y"])));
    assert!(doc == doc.replace(5000, 5001, &doc.slice(5000, 5001)));
    assert!(doc != doc.replace(5000, 5001, &of(&["y"])));
}

#[test]
fn can_be_compared_despite_different_tree_shape() {
    let mut expected = vec![line() + "abc"];
    expected.extend(lines()[2..].iter().cloned());
    assert!(doc0().replace(100, 201, &of(&["abc"])) == Text::of(&expected).unwrap());
}

#[test]
fn can_compare_small_documents() {
    assert!(of(&["foo", "bar"]) == of(&["foo", "bar"]));
    assert!(of(&["foo", "bar"]) != of(&["foo", "baz"]));
}

#[test]
fn is_iterable() {
    let doc = doc0();
    let mut iter = doc.iter(1);
    let mut build = String::new();
    loop {
        iter.advance(0);
        if iter.done() {
            assert_eq!(build, text0());
            break;
        }
        if iter.line_break() {
            build.push('\n');
        } else {
            assert!(!iter.value().contains('\n'));
            build += iter.value();
        }
    }
}

#[test]
fn is_iterable_in_reverse() {
    let doc = doc0();
    let mut found = String::new();
    let mut iter = doc.iter(-1);
    while !iter.advance(0).done() {
        found.insert_str(0, iter.value());
    }
    assert_eq!(found, text0());
}

#[test]
fn allows_negative_skip_values_in_iteration() {
    let doc = of(&["one", "two", "three", "four"]);
    let mut iter = doc.iter(1);
    assert_eq!(iter.advance(12).value(), "e");
    assert_eq!(iter.advance(-12).value(), "ne");
    assert_eq!(iter.advance(12).value(), "our");
    assert_eq!(iter.advance(-1000).value(), "one");
}

#[test]
fn is_partially_iterable() {
    let doc = doc0();
    let found: String = doc.iter_range(500, doc.len() - 500).collect();
    let text0 = text0();
    assert_eq!(found, &text0[500..text0.len() - 500]);
}

#[test]
fn is_partially_iterable_in_reverse() {
    let doc = doc0();
    let mut found = String::new();
    let mut iter = doc.iter_range(doc.len() - 500, 500);
    while !iter.advance(0).done() {
        found.insert_str(0, iter.value());
    }
    let text0 = text0();
    assert_eq!(found, &text0[500..text0.len() - 500]);
}

#[test]
fn can_partially_iter_over_subsections_at_the_start_and_end() {
    let doc = doc0();
    let len = doc.len();
    assert_eq!(doc.iter_range(0, 1).advance(0).value(), "1");
    assert_eq!(doc.iter_range(1, 2).advance(0).value(), "2");
    assert_eq!(doc.iter_range(len - 1, len).advance(0).value(), "0");
    assert_eq!(doc.iter_range(len - 2, len - 1).advance(0).value(), "9");
}

#[test]
fn can_iterate_over_lines() {
    let doc = of(&["ab", "cde", "", "", "f", "", "g"]);
    let get = |from: Option<usize>, to: Option<usize>| -> String {
        doc.iter_lines(from, to).collect::<Vec<_>>().join("\n")
    };
    assert_eq!(get(None, None), "ab\ncde\n\n\nf\n\ng");
    assert_eq!(get(Some(1), Some(doc.lines() + 1)), "ab\ncde\n\n\nf\n\ng");
    assert_eq!(get(Some(2), Some(3)), "cde");
    assert_eq!(get(Some(1), Some(5)), "ab\ncde\n\n");
    assert_eq!(get(Some(2), Some(1)), "");
    assert_eq!(get(Some(3), None), "\n\nf\n\ng");
}

#[test]
fn can_convert_to_json() {
    let mut lines = lines();
    for i in 0..200 {
        lines.push(format!("line {i}"));
    }
    let text = Text::of(&lines).unwrap();
    assert!(Text::of(&text.to_json()).unwrap() == text);
}

#[test]
#[should_panic(expected = "Invalid line")]
fn rejects_line_zero() {
    doc0().line(0);
}

#[test]
#[should_panic(expected = "Invalid line")]
fn rejects_a_line_past_the_end() {
    let doc = doc0();
    doc.line(doc.lines() + 1);
}

#[test]
fn can_get_line_info_by_line_number() {
    let doc = doc0();
    for i in (1..doc.lines()).step_by(5) {
        let l = doc.line(i);
        assert_eq!(l.from, (i - 1) * 101);
        assert_eq!(l.to, i * 101 - 1);
        assert_eq!(l.number, i);
        assert_eq!(l.text, line());
    }
}

#[test]
#[should_panic(expected = "Invalid position")]
fn rejects_a_position_past_the_end() {
    let doc = doc0();
    doc.line_at(doc.len() + 1);
}

#[test]
fn can_get_line_info_by_position() {
    let doc = doc0();
    for i in (0..doc.len()).step_by(5) {
        let l = doc.line_at(i);
        assert_eq!(l.from, i - (i % 101));
        assert_eq!(l.to, i - (i % 101) + 100);
        assert_eq!(l.number, i / 101 + 1);
        assert_eq!(l.text, line());
    }
}

#[test]
fn can_delete_a_range_at_the_start_of_a_child_node() {
    assert_eq!(
        doc0().replace(0, 100, &of(&["x"])).to_string(),
        format!("x{}", &text0()[100..])
    );
}

#[test]
fn can_retrieve_pieces_of_text() {
    let doc = doc0();
    let text0 = text0();
    for _ in 0..500 {
        let from = r(doc.len() - 1);
        let to = if coin() {
            from + 2
        } else {
            from + r(doc.len() - 1 - from) + 1
        };
        assert_eq!(doc.slice_string(from, to), &text0[from..to]);
        assert_eq!(doc.slice(from, to).to_string(), &text0[from..to]);
    }
}

#[test]
fn clips_out_of_range_boundaries() {
    // Upstream also passes negative positions, which `usize` rules out.
    assert_eq!(Text::empty().slice(0, 10).len(), 0);
    assert_eq!(Text::empty().slice(1000, 1100).len(), 0);
    assert_eq!(doc0().slice(5, 0).len(), 0);
}

#[test]
fn rejects_a_document_without_lines() {
    let none: [&str; 0] = [];
    assert!(Text::of(&none).is_err());
}

#[test]
fn splits_strings_at_every_line_break() {
    let doc = Text::from("a\r\nb\rc\nd");
    assert_eq!(doc.lines(), 4);
    assert_eq!(doc.to_string(), "a\nb\nc\nd");
}

#[test]
fn counts_positions_in_utf8_bytes() {
    // The port's one deliberate difference: positions are UTF-8 offsets.
    let doc = Text::from("héllo\nwörld");
    assert_eq!(doc.len(), "héllo\nwörld".len());
    assert_eq!(doc.line(2).from, "héllo\n".len());
    assert_eq!(doc.slice_string(1, 3), "é");
}
