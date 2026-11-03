//! Runs the html5lib tree construction tests when `RON_HTML5LIB_DIR` names a directory of
//! `.dat` files, the last version of which is in the history of html5lib-tests:
//! `RON_HTML5LIB_DIR=... cargo test --release --test html5lib -- --ignored --nocapture`
//! With `RON_HTML5LIB_REPORT` naming a file, failures are written there in full.

use std::fmt::Write as _;
use std::panic::catch_unwind;

use ron::html::{Document, Namespace, NodeId, NodeKind, parse};

/// What still fails: `<selectedcontent>` copies of the selected option, which has no use
/// without scripting, and a newline after `<pre>` that is written as a character reference.
const PASSING_CASES: usize = 1587;

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
        let lines = || rest.split('\n').take_while(|line| *line != "#document");
        if lines().any(|line| matches!(line, "#document-fragment" | "#script-on")) {
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

fn doctype_text(document: &Document, id: NodeId) -> String {
    let doctype = document.doctype(id);
    if doctype.public_id.is_none() && doctype.system_id.is_none() {
        return format!("<!DOCTYPE {}>", doctype.name);
    }
    format!(
        "<!DOCTYPE {} \"{}\" \"{}\">",
        doctype.name,
        doctype.public_id.unwrap_or_default(),
        doctype.system_id.unwrap_or_default()
    )
}

/// The attribute name as the test files show it. ron keeps names as written, so the
/// mapping of the standard for foreign elements lives here until something needs it.
fn attribute_name(name: &str, namespace: Namespace) -> String {
    let lower = name.to_ascii_lowercase();
    if namespace == Namespace::Html {
        return lower;
    }
    if namespace == Namespace::MathMl && lower == "definitionurl" {
        return "definitionURL".to_owned();
    }
    if namespace == Namespace::Svg
        && let Some(mixed) = SVG_ATTRIBUTES
            .iter()
            .find(|mixed| mixed.eq_ignore_ascii_case(&lower))
    {
        return (*mixed).to_owned();
    }
    match lower.as_str() {
        "xlink:actuate" | "xlink:arcrole" | "xlink:href" | "xlink:role" | "xlink:show"
        | "xlink:title" | "xlink:type" | "xml:lang" | "xml:space" | "xmlns:xlink" => {
            lower.replacen(':', " ", 1)
        }
        _ => lower,
    }
}

const SVG_ATTRIBUTES: [&str; 58] = [
    "attributeName",
    "attributeType",
    "baseFrequency",
    "baseProfile",
    "calcMode",
    "clipPathUnits",
    "diffuseConstant",
    "edgeMode",
    "filterUnits",
    "glyphRef",
    "gradientTransform",
    "gradientUnits",
    "kernelMatrix",
    "kernelUnitLength",
    "keyPoints",
    "keySplines",
    "keyTimes",
    "lengthAdjust",
    "limitingConeAngle",
    "markerHeight",
    "markerUnits",
    "markerWidth",
    "maskContentUnits",
    "maskUnits",
    "numOctaves",
    "pathLength",
    "patternContentUnits",
    "patternTransform",
    "patternUnits",
    "pointsAtX",
    "pointsAtY",
    "pointsAtZ",
    "preserveAlpha",
    "preserveAspectRatio",
    "primitiveUnits",
    "refX",
    "refY",
    "repeatCount",
    "repeatDur",
    "requiredExtensions",
    "requiredFeatures",
    "specularConstant",
    "specularExponent",
    "spreadMethod",
    "startOffset",
    "stdDeviation",
    "stitchTiles",
    "surfaceScale",
    "systemLanguage",
    "tableValues",
    "targetX",
    "targetY",
    "textLength",
    "viewBox",
    "viewTarget",
    "xChannelSelector",
    "yChannelSelector",
    "zoomAndPan",
];

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
                        *pending = (!text.is_empty()).then_some((depth, text));
                    }
                }
            }
            NodeKind::Element => {
                flush(&mut pending_text, &mut out);
                let prefix = match document.namespace(id) {
                    Namespace::Html => "",
                    Namespace::Svg => "svg ",
                    Namespace::MathMl => "math ",
                };
                writeln!(out, "| {indent}<{prefix}{}>", document.local_name(id)).unwrap();
                let mut attributes: Vec<(String, String)> = Vec::new();
                for attribute in document.attributes(id) {
                    let name = attribute_name(attribute.name, document.namespace(id));
                    if attributes.iter().all(|(other, _)| *other != name) {
                        let value = attribute.decoded_value();
                        attributes.push((name, value.into_owned()));
                    }
                }
                attributes.sort();
                for (name, value) in attributes {
                    writeln!(out, "| {indent}  {name}=\"{value}\"").unwrap();
                }
            }
            NodeKind::Comment => {
                flush(&mut pending_text, &mut out);
                writeln!(out, "| {indent}<!-- {} -->", document.comment_text(id)).unwrap();
            }
            NodeKind::Doctype => {
                flush(&mut pending_text, &mut out);
                writeln!(out, "| {indent}{}", doctype_text(document, id)).unwrap();
            }
            NodeKind::Fragment => {
                flush(&mut pending_text, &mut out);
                writeln!(out, "| {indent}content").unwrap();
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
    assert!(passed >= PASSING_CASES, "only {passed} cases pass");
}
