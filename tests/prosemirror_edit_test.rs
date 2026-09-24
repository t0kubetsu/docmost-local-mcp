use docmost_local_mcp::prosemirror::{markdown_to_prosemirror, prosemirror_to_markdown};
use serde_json::{Value, json};

fn comment_mark(id: &str) -> Value {
    json!({ "type": "comment", "attrs": { "commentId": id, "resolved": false } })
}

fn has_comment(value: &Value, id: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.get("type").and_then(Value::as_str) == Some("comment")
                && map["attrs"]["commentId"] == id
                || map.values().any(|v| has_comment(v, id))
        }
        Value::Array(items) => items.iter().any(|v| has_comment(v, id)),
        _ => false,
    }
}

/// The defect `edit_page` exists for: `update_page` rebuilds the body from Markdown, which has
/// no form for a comment mark, so every inline comment thread on the page is detached.
#[test]
fn markdown_round_trip_drops_comment_marks() {
    let doc = json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [
        { "type": "text", "text": "Caddy or nginx (QUIC)", "marks": [comment_mark("c1")] }
    ]}]});
    assert!(has_comment(&doc, "c1"));
    let round_trip = markdown_to_prosemirror(&prosemirror_to_markdown(&doc));
    assert!(!has_comment(&round_trip, "c1"));
}

use docmost_local_mcp::{prosemirror::apply_edits, types::EditOperation};

fn text(t: &str, marks: Value) -> Value {
    json!({ "type": "text", "text": t, "marks": marks })
}

fn plain(t: &str) -> Value {
    json!({ "type": "text", "text": t })
}

fn para(content: Value) -> Value {
    json!({ "type": "paragraph", "attrs": { "textAlign": null }, "content": content })
}

fn cell(kind: &str, t: &str) -> Value {
    json!({ "type": kind, "attrs": { "colspan": 1, "rowspan": 1, "colwidth": [120] },
            "content": [para(json!([plain(t)]))] })
}

fn row(kind: &str, cells: &[&str]) -> Value {
    json!({ "type": "tableRow", "content": cells.iter().map(|t| cell(kind, t)).collect::<Vec<_>>() })
}

fn ops(value: Value) -> Vec<EditOperation> {
    serde_json::from_value(value).expect("valid operations")
}

/// A page with every node kind the edit must leave alone.
fn page() -> Value {
    json!({ "type": "doc", "content": [
        { "type": "heading", "attrs": { "level": 2, "id": "h1" }, "content": [plain("IDR-023")] },
        para(json!([
            plain("The only host rule is the ceiling, which "),
            text("blocks no range", json!([{ "type": "bold" }])),
            plain(". Use "),
            text("Caddy or nginx (QUIC)", json!([comment_mark("c1")])),
            plain(" as proxy, ask "),
            { "type": "mention", "attrs": { "id": "m1", "label": "Alice", "entityType": "user", "entityId": "u1" } },
            plain("."),
        ])),
        { "type": "callout", "attrs": { "type": "info" }, "content": [para(json!([plain("RabbitMQ note")]))] },
        { "type": "codeBlock", "attrs": { "language": "mermaid" }, "content": [plain("graph TD\n  WIN -->|RFC1918 blocked| NET")] },
        { "type": "table", "content": [
            row("tableHeader", &["Version", "Date", "Change"]),
            row("tableCell", &["0.26", "2026-09-20", "Older"]),
            { "type": "tableRow", "content": [
                cell("tableCell", "0.27"), cell("tableCell", "2026-09-23"),
                { "type": "tableCell", "attrs": { "colspan": 1, "rowspan": 1, "colwidth": [300] },
                  "content": [para(json!([text("on password change", json!([comment_mark("c2")]))]))] },
            ]},
        ]},
    ]})
}

fn block(doc: &Value, index: usize) -> &Value {
    &doc["content"][index]
}

#[test]
fn replace_keeps_every_other_node_and_mark() {
    let before = page();
    let outcome = apply_edits(&before, &ops(json!([
        { "op": "replace_text", "find": "The only host rule is", "replace": "The only host rule was" }
    ]))).unwrap();
    for index in [0, 2, 3, 4] {
        assert_eq!(
            block(&outcome.doc, index),
            block(&before, index),
            "block {index} changed"
        );
    }
    let (old, new) = (
        &block(&before, 1)["content"],
        &block(&outcome.doc, 1)["content"],
    );
    assert_eq!(new[0], plain("The only host rule was"));
    assert_eq!(new[1], plain(" the ceiling, which "));
    for index in 1..old.as_array().unwrap().len() {
        assert_eq!(new[index + 1], old[index], "inline node {index} changed");
    }
    assert!(outcome.detached_comments.is_empty());
    assert_eq!(outcome.unchanged_blocks, (4, 5));
}

#[test]
fn span_inside_one_commented_text_node_keeps_the_comment() {
    let outcome = apply_edits(
        &page(),
        &ops(json!([
            { "op": "replace_text", "find": "nginx", "replace": "HAProxy" }
        ])),
    )
    .unwrap();
    let inline = &block(&outcome.doc, 1)["content"];
    let marks = json!([comment_mark("c1")]);
    assert_eq!(inline[3], text("Caddy or ", marks.clone()));
    assert_eq!(inline[4], text("HAProxy", marks.clone()));
    assert_eq!(inline[5], text(" (QUIC)", marks));
    assert!(outcome.detached_comments.is_empty());
}

#[test]
fn span_across_marks_is_replaced_with_the_new_marks() {
    let outcome = apply_edits(&page(), &ops(json!([
        { "op": "replace_text", "find": "which **blocks no range**. Use", "replace": "which **blocked nothing**. Use" }
    ]))).unwrap();
    let inline = &block(&outcome.doc, 1)["content"];
    assert_eq!(inline[0], plain("The only host rule is the ceiling, "));
    assert_eq!(inline[1], plain("which "));
    assert_eq!(
        inline[2],
        text("blocked nothing", json!([{ "type": "bold" }]))
    );
    assert_eq!(inline[3], plain(". Use"));
    assert_eq!(inline[4], plain(" "));
    assert_eq!(inline[5], block(&page(), 1)["content"][3]);
}

#[test]
fn span_inside_bold_inherits_bold() {
    let outcome = apply_edits(
        &page(),
        &ops(json!([
            { "op": "replace_text", "find": "no range", "replace": "every range" }
        ])),
    )
    .unwrap();
    let inline = &block(&outcome.doc, 1)["content"];
    assert_eq!(inline[1], text("blocks ", json!([{ "type": "bold" }])));
    assert_eq!(inline[2], text("every range", json!([{ "type": "bold" }])));
}

#[test]
fn find_that_cuts_mark_syntax_fails() {
    let error = apply_edits(
        &page(),
        &ops(json!([
            { "op": "replace_text", "find": "*blocks no", "replace": "x" }
        ])),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("inside Markdown syntax"),
        "{error}"
    );
}

#[test]
fn zero_or_two_matches_fail_with_the_count() {
    let zero = apply_edits(
        &page(),
        &ops(json!([{ "op": "replace_text", "find": "absent", "replace": "x" }])),
    );
    assert!(zero.unwrap_err().to_string().contains("matches 0 times"));
    let two = apply_edits(
        &page(),
        &ops(json!([{ "op": "replace_text", "find": "RFC1918", "replace": "x" }])),
    );
    assert!(two.is_ok(), "only the mermaid block has RFC1918");
    let two = apply_edits(
        &page(),
        &ops(json!([{ "op": "replace_text", "find": "0.2", "replace": "x" }])),
    );
    assert!(two.unwrap_err().to_string().contains("matches 2 times"));
}

#[test]
fn a_failing_later_operation_fails_the_whole_edit() {
    let error = apply_edits(
        &page(),
        &ops(json!([
            { "op": "replace_text", "find": "The only", "replace": "One" },
            { "op": "replace_text", "find": "absent", "replace": "x" }
        ])),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("operation 2 failed, nothing written"),
        "{error}"
    );
}

#[test]
fn partial_overlap_with_a_comment_is_reported_as_detached() {
    let outcome = apply_edits(&page(), &ops(json!([
        { "op": "replace_text", "find": ". Use Caddy or nginx (QUIC) as", "replace": ". Use Caddy as" }
    ]))).unwrap();
    assert_eq!(outcome.detached_comments, vec!["c1".to_string()]);
}

#[test]
fn append_table_row_adds_a_fresh_row_and_keeps_the_others() {
    let before = page();
    let outcome = apply_edits(&before, &ops(json!([
        { "op": "append_table_row", "anchor": "| 0.27 |", "row": "| 0.28 | 2026-09-24 | **IDR-025** added |" }
    ]))).unwrap();
    let (old, new) = (
        &block(&before, 4)["content"],
        &block(&outcome.doc, 4)["content"],
    );
    assert_eq!(new.as_array().unwrap().len(), 4);
    for index in 0..3 {
        assert_eq!(new[index], old[index], "row {index} changed");
    }
    let added = &new[3]["content"];
    assert_eq!(added[0]["type"], "tableCell");
    assert_eq!(added[2]["attrs"]["colwidth"], json!([300]));
    assert_eq!(
        added[2]["content"][0]["content"][0],
        text("IDR-025", json!([{ "type": "bold" }]))
    );
    assert!(
        !has_comment(&new[3], "c2"),
        "a comment mark must never be copied"
    );
}

#[test]
fn whole_row_replace_keeps_unchanged_cells_and_adds_rows() {
    let before = page();
    let outcome = apply_edits(
        &before,
        &ops(json!([
            { "op": "replace_text",
              "find": "| 0.27 | 2026-09-23 | on password change |",
              "replace": "| 0.27 | 2026-09-24 | on password change |\n| 0.28 | 2026-09-24 | new |" }
        ])),
    )
    .unwrap();
    let (old, new) = (
        &block(&before, 4)["content"][2]["content"],
        &block(&outcome.doc, 4)["content"],
    );
    assert_eq!(new[2]["content"][0], old[0]);
    assert_eq!(
        new[2]["content"][2], old[2],
        "the commented cell must stay as is"
    );
    assert_eq!(
        new[2]["content"][1]["content"][0]["content"][0],
        plain("2026-09-24")
    );
    assert_eq!(
        new[3]["content"][2]["content"][0]["content"][0],
        plain("new")
    );
    assert!(outcome.detached_comments.is_empty());
}

#[test]
fn row_with_wrong_cell_count_fails() {
    let error = apply_edits(
        &page(),
        &ops(json!([
            { "op": "append_table_row", "anchor": "| 0.27 |", "row": "| 0.28 | too few |" }
        ])),
    )
    .unwrap_err();
    assert!(error.to_string().contains("3 cells"), "{error}");
}

#[test]
fn insert_blocks_lands_after_the_anchor_block() {
    let before = page();
    let outcome = apply_edits(&before, &ops(json!([
        { "op": "insert_blocks", "anchor": "IDR-023", "position": "after", "markdown": "### IDR-025 — New\n\nBody **text**." }
    ]))).unwrap();
    let content = outcome.doc["content"].as_array().unwrap();
    assert_eq!(content.len(), 7);
    assert_eq!(content[0], *block(&before, 0));
    assert_eq!(content[1]["type"], "heading");
    assert_eq!(content[2]["type"], "paragraph");
    assert_eq!(&content[3..], &before["content"].as_array().unwrap()[1..]);
    let ambiguous = apply_edits(
        &before,
        &ops(json!([
            { "op": "insert_blocks", "anchor": "0.2", "position": "before", "markdown": "x" }
        ])),
    );
    assert!(
        ambiguous
            .unwrap_err()
            .to_string()
            .contains("matches 2 times")
    );
}

#[test]
fn code_block_replace_is_literal() {
    let outcome = apply_edits(
        &page(),
        &ops(json!([
            { "op": "replace_text", "find": "RFC1918 blocked", "replace": "**non-global** dropped" }
        ])),
    )
    .unwrap();
    assert_eq!(
        block(&outcome.doc, 3)["content"],
        json!([
            plain("graph TD\n  WIN -->|"),
            plain("**non-global** dropped"),
            plain("| NET")
        ])
    );
}

#[test]
fn row_appended_after_a_header_row_is_a_body_row() {
    let outcome = apply_edits(&page(), &ops(json!([
        { "op": "append_table_row", "anchor": "| Version |", "row": "| 0.1 | 2026-01-01 | First |" }
    ]))).unwrap();
    let added = &block(&outcome.doc, 4)["content"][1]["content"];
    assert!(
        added
            .as_array()
            .unwrap()
            .iter()
            .all(|cell| cell["type"] == "tableCell")
    );
}

use docmost_local_mcp::prosemirror::stored_matches_sent;

/// Measured 2026-09-24: the server stores new nodes with default attrs the writer never sets.
#[test]
fn stored_copy_with_server_defaults_matches_what_was_sent() {
    let sent = json!({ "type": "doc", "content": [
        { "type": "heading", "attrs": { "level": 3 }, "content": [plain("IDR-025")] },
        { "type": "paragraph", "content": [
            text("Taiga", json!([{ "type": "link", "attrs": { "href": "https://x.test/1" } }])),
            text("bold", json!([{ "type": "bold" }])),
        ]},
    ]});
    let stored = json!({ "type": "doc", "content": [
        { "type": "heading", "attrs": { "level": 3, "indent": 0 }, "content": [plain("IDR-025")] },
        { "type": "paragraph", "attrs": { "indent": 0 }, "content": [
            text("Taiga", json!([{ "type": "link", "attrs": { "href": "https://x.test/1", "target": "_blank",
                "rel": "noopener noreferrer nofollow", "class": null, "title": null, "internal": false } }])),
            text("bold", json!([{ "type": "bold", "attrs": {} }])),
        ]},
    ]});
    assert!(stored_matches_sent(&sent, &stored));

    let mut changed_text = stored.clone();
    changed_text["content"][1]["content"][1]["text"] = json!("bolder");
    assert!(!stored_matches_sent(&sent, &changed_text));

    let mut changed_attr = stored.clone();
    changed_attr["content"][0]["attrs"]["level"] = json!(2);
    assert!(!stored_matches_sent(&sent, &changed_attr));

    let mut extra_block = stored.clone();
    extra_block["content"]
        .as_array_mut()
        .unwrap()
        .insert(0, json!({ "type": "paragraph", "attrs": { "indent": 0 } }));
    assert!(
        !stored_matches_sent(&sent, &extra_block),
        "an editor's empty paragraph must be reported"
    );

    let mut dropped_mark = stored.clone();
    dropped_mark["content"][1]["content"][1]["marks"] = json!([]);
    assert!(!stored_matches_sent(&sent, &dropped_mark));
}
