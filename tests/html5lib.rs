//! Runs the html5lib tree construction tests when `RON_HTML5LIB_DIR` names a directory of
//! `.dat` files, the last version of which is in the history of html5lib-tests:
//! `RON_HTML5LIB_DIR=... cargo test --release --test html5lib -- --ignored --nocapture`
//! With `RON_HTML5LIB_REPORT` naming a file, failures are written there in full.

use std::fmt::Write as _;
use std::panic::catch_unwind;

use ron::html::{Document, NodeId, NodeKind, parse};

struct Case {
    data: String,
    expected: String,
}

/// Reads the cases of a file, leaving out fragment parsing and scripting, which ron lacks.
fn read_cases(text: &str) -> (Vec<Case>, usize) {
    let mut cases = Vec::new();
    let mut skipped = 0;
    for block in text.split("#data\n").skip(1) {
        let Some((data, rest)) = block.split_once("\n#errors\n") else {
            continue;
        };
        if rest.contains("\n#document-fragment\n") || rest.contains("\n#script-on\n") {
            skipped += 1;
            continue;
        }
        let Some((_, document)) = rest.split_once("#document\n") else {
            continue;
        };
        cases.push(Case {
            data: data.to_owned(),
            expected: document.trim_end_matches('\n').to_owned(),
        });
    }
    (cases, skipped)
}

fn comment_text(raw: &str) -> &str {
    if let Some(body) = raw.strip_prefix("<!--") {
        let body = body
            .strip_suffix("-->")
            .or(body.strip_suffix("--!>"))
            .unwrap_or(body);
        return body
            .strip_suffix("--")
            .or(body.strip_suffix('-'))
            .unwrap_or(body);
    }
    let body = raw.strip_suffix('>').unwrap_or(raw);
    body.strip_prefix("</")
        .or(body.strip_prefix("<!"))
        .or(body.strip_prefix('<'))
        .unwrap_or(body)
}

fn doctype_text(raw: &str) -> String {
    let body = raw.strip_suffix('>').unwrap_or(raw).get(9..).unwrap_or("");
    let body = body.trim_matches(|c: char| c.is_ascii_whitespace());
    let mut words = body.splitn(2, |c: char| c.is_ascii_whitespace());
    let name = words.next().unwrap_or("").to_ascii_lowercase();
    let rest = words.next().unwrap_or("").trim();
    let upper = rest.to_ascii_uppercase();
    let quoted: Vec<&str> = rest.split(['"', '\'']).skip(1).step_by(2).collect();
    if upper.starts_with("PUBLIC") {
        let public = quoted.first().copied().unwrap_or("");
        let system = quoted.get(1).copied().unwrap_or("");
        format!("<!DOCTYPE {name} \"{public}\" \"{system}\">")
    } else if upper.starts_with("SYSTEM") {
        let system = quoted.first().copied().unwrap_or("");
        format!("<!DOCTYPE {name} \"\" \"{system}\">")
    } else {
        format!("<!DOCTYPE {name}>")
    }
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Writes a tree in the format of the test files. Adjacent text nodes count as one.
fn serialize(document: &Document) -> String {
    let root = document.root();
    let mut out = String::new();
    let mut pending_text: Option<(usize, String)> = None;
    let flush = |pending: &mut Option<(usize, String)>, out: &mut String| {
        if let Some((depth, text)) = pending.take() {
            writeln!(out, "| {}\"{text}\"", "  ".repeat(depth)).unwrap();
        }
    };
    let depth_of = |id: NodeId| {
        std::iter::successors(document.parent(id), |&parent| document.parent(parent)).count() - 1
    };
    let mut previous: Option<NodeId> = None;
    for id in document.descendants(root).skip(1) {
        let depth = depth_of(id);
        let indent = "  ".repeat(depth);
        match document.kind(id) {
            NodeKind::Text => {
                let text = document.decoded_text(id).into_owned();
                let adjoins = previous
                    .is_some_and(|before| document.next_sibling(before) == Some(id))
                    && previous.is_some_and(|before| document.kind(before) == NodeKind::Text);
                match &mut pending_text {
                    Some((_, joined)) if adjoins => joined.push_str(&text),
                    pending => {
                        flush(pending, &mut out);
                        *pending = Some((depth, text));
                    }
                }
            }
            NodeKind::Element => {
                flush(&mut pending_text, &mut out);
                writeln!(
                    out,
                    "| {indent}<{}>",
                    document.name(id).to_ascii_lowercase()
                )
                .unwrap();
                let mut attributes: Vec<(String, String)> = Vec::new();
                for attribute in document.attributes(id) {
                    let name = attribute.name.to_ascii_lowercase();
                    if attributes.iter().all(|(other, _)| *other != name) {
                        let value = attribute.decoded_value();
                        attributes.push((name, normalize_newlines(&value)));
                    }
                }
                attributes.sort();
                for (name, value) in attributes {
                    writeln!(out, "| {indent}  {name}=\"{value}\"").unwrap();
                }
            }
            NodeKind::Comment => {
                flush(&mut pending_text, &mut out);
                let text = normalize_newlines(comment_text(document.text(id)));
                writeln!(out, "| {indent}<!-- {text} -->").unwrap();
            }
            NodeKind::Doctype => {
                flush(&mut pending_text, &mut out);
                writeln!(out, "| {indent}{}", doctype_text(document.text(id))).unwrap();
            }
            NodeKind::Document => {}
        }
        previous = Some(id);
    }
    flush(&mut pending_text, &mut out);
    out.trim_end_matches('\n').to_owned()
}

#[test]
#[ignore = "needs RON_HTML5LIB_DIR"]
fn html5lib_tree_construction() {
    let directory = std::env::var("RON_HTML5LIB_DIR").expect("RON_HTML5LIB_DIR must be set");
    let report_path = std::env::var("RON_HTML5LIB_REPORT").ok();
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "dat"))
        .collect();
    paths.sort();
    let (mut passed, mut total, mut skipped, mut panics) = (0, 0, 0, 0);
    let mut report = String::new();
    for path in &paths {
        let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
        let (cases, file_skipped) = read_cases(&text);
        skipped += file_skipped;
        let mut file_passed = 0;
        for case in &cases {
            let data = case.data.clone();
            let Ok(actual) = catch_unwind(move || serialize(&parse(&data))) else {
                panics += 1;
                writeln!(report, "=== PANIC {path:?}\n{}\n", case.data).unwrap();
                continue;
            };
            if actual == case.expected {
                file_passed += 1;
            } else {
                writeln!(
                    report,
                    "=== {:?}\n{}\n--- expected\n{}\n--- actual\n{actual}\n",
                    path.file_name().unwrap(),
                    case.data,
                    case.expected
                )
                .unwrap();
            }
        }
        println!(
            "{:40} {file_passed:4} / {:4}",
            path.file_name().unwrap().to_string_lossy(),
            cases.len()
        );
        passed += file_passed;
        total += cases.len();
    }
    println!("passed {passed} / {total}, panics {panics}, skipped {skipped}");
    if let Some(report_path) = report_path {
        std::fs::write(report_path, report).unwrap();
    }
    assert!(total > 0, "no test cases found");
    assert_eq!(panics, 0);
}
