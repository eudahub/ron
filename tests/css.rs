//! Stylesheet parsing, selector validity and selector matching.

use ron::css::{Stylesheet, matches};
use ron::html::{Document, NodeId, Tag, parse};

fn sheet(source: &str) -> Stylesheet {
    Stylesheet::parse(source)
}

/// Returns the declarations of a sheet as `name=value` in order, marking `!important`.
fn declarations(source: &str) -> Vec<String> {
    let stylesheet = sheet(source);
    stylesheet
        .rules
        .iter()
        .flat_map(|rule| &rule.declarations)
        .map(|declaration| {
            let mark = if declaration.important { "!" } else { "" };
            format!(
                "{}={}{mark}",
                declaration.name,
                stylesheet.value(declaration)
            )
        })
        .collect()
}

fn is_valid_selector(selector: &str) -> bool {
    !sheet(&format!("{selector} {{ color: red }}"))
        .rules
        .is_empty()
}

fn find(document: &Document, wanted: &str) -> NodeId {
    document
        .descendants(document.root())
        .find(|&id| {
            document
                .attributes(id)
                .any(|attribute| attribute.name == "t" && attribute.value == Some(wanted))
        })
        .expect("element with this t attribute")
}

/// Whether the first selector of a rule matches the element marked `t=wanted`.
fn selects(selector: &str, document: &Document, wanted: &str) -> bool {
    let stylesheet = sheet(&format!("{selector} {{ color: red }}"));
    let rule = stylesheet.rules.first().expect("valid selector");
    rule.selectors
        .iter()
        .any(|selector| matches(document, find(document, wanted), selector))
}

#[test]
fn reads_rules_and_declarations() {
    assert_eq!(
        declarations("p, div > a { color : red ; margin: 0 auto !important; } b{top:1px}"),
        ["color=red", "margin=0 auto!", "top=1px"]
    );
    assert_eq!(
        declarations("a { COLOR: Red; --Own: { x } }"),
        ["color=Red"]
    );
    assert_eq!(declarations("a { --Own: 1 }"), ["--Own=1"]);
    assert_eq!(
        declarations("/* c */ a /* d */ { /* e */ top /* f */ : /* g */ 1 /* h */ }"),
        ["top=1"]
    );
    assert_eq!(declarations("<!-- a { top: 1 } -->"), ["top=1"]);
}

#[test]
fn errors_drop_only_what_they_touch() {
    assert_eq!(
        declarations("a { top 1; left: 2; : 3; right: 4 }"),
        ["left=2", "right=4"]
    );
    assert_eq!(declarations("a { top: ; left: 2 }"), ["left=2"]);
    assert_eq!(
        declarations("a[ { top: 1 } b { left: 2 }"),
        Vec::<String>::new()
    );
    assert_eq!(declarations("a?b { top: 1 } c { left: 2 }"), ["left=2"]);
    assert_eq!(declarations("a, 1b { top: 1 } c { left: 2 }"), ["left=2"]);
    assert_eq!(declarations("a { b { c: 1 } left: 2 }"), ["left=2"]);
    assert_eq!(
        declarations("@unknown x { a { top: 1 } } b { left: 2 }"),
        ["left=2"]
    );
    assert_eq!(declarations("@charset 'x'; a { left: 2 }"), ["left=2"]);
    assert_eq!(
        declarations("a { left: url(x;y); top: \"}\"; right: f(;) }"),
        ["left=url(x;y)", "top=\"}\"", "right=f(;)"]
    );
}

#[test]
fn input_ending_early_closes_what_is_open() {
    assert_eq!(declarations("a { top: 1"), ["top=1"]);
    assert_eq!(declarations("a { top: f(1"), ["top=f(1"]);
    assert_eq!(declarations("a { top: \"x"), ["top=\"x"]);
    assert_eq!(declarations("a { /* top: 1"), Vec::<String>::new());
    assert_eq!(declarations("a"), Vec::<String>::new());
    assert_eq!(declarations("@media print { a { top: 1"), ["top=1"]);
}

#[test]
fn keeps_media_conditions_and_imports() {
    let stylesheet = sheet(
        "@import url(a.css); @import 'b.css' print; @media (min-width: 1px) { a { top: 1 } @media print { b { top: 2 } } } c { top: 3 }",
    );
    assert_eq!(&*stylesheet.imports, ["a.css".into(), "b.css".into()]);
    assert_eq!(
        &*stylesheet.media_queries,
        ["(min-width: 1px)".into(), "print".into()]
    );
    let media: Vec<_> = stylesheet.rules.iter().map(|rule| rule.media).collect();
    assert_eq!(media, [Some(0), Some(1), None]);
}

#[test]
fn grammar_decides_selector_validity() {
    for valid in [
        "a",
        "*",
        ".a",
        "#a",
        "a.b#c",
        "a b",
        "a>b",
        "a > b",
        "a + b",
        "a ~ b",
        "a,b",
        "a , b",
        "[a]",
        "[ a = b ]",
        "[a=\"b\"]",
        "[a~=b]",
        "[a|=b]",
        "[a^=b]",
        "[a$=b]",
        "[a*=b]",
        "[a=b i]",
        ":hover",
        "a:first-child",
        "::before",
        "a::after",
        ":before",
        "a:not(b)",
        ":is(a, b)",
        ":where(a)",
        ":nth-child(2n + 1)",
        ":nth-child(odd)",
        ":nth-last-of-type(3)",
        "a\\:b",
        "\u{17c}",
        "-a",
        "--a",
        "a  b",
        "/* c */ a /* d */ b",
    ] {
        assert!(is_valid_selector(valid), "{valid:?} should be valid");
    }
    for invalid in [
        "",
        ",",
        "a,",
        ",a",
        "a,,b",
        ">",
        "a>",
        ">a",
        "a>>b",
        "a b>",
        "1a",
        "#1a",
        ".",
        ".1",
        "a.",
        "[",
        "[]",
        "[a",
        "[a=]",
        "[=b]",
        "[a=1]",
        "[a=b c]",
        ":",
        ":unknown",
        "::unknown",
        ":not(",
        ":not()",
        ":not(1)",
        ":nth-child(x)",
        ":unknown(a)",
        "a::before b",
        "a{",
        "a;b",
        "a?",
        "a/b",
    ] {
        assert!(!is_valid_selector(invalid), "{invalid:?} should be invalid");
    }
}

#[test]
fn specificity_orders_ids_classes_and_types() {
    let specificity = |selector: &str| {
        sheet(&format!("{selector} {{ top: 1 }}")).rules[0].selectors[0].specificity
    };
    assert!(specificity("#a") > specificity(".a.b.c.d"));
    assert!(specificity(".a") > specificity("a b c d"));
    assert!(specificity("a b") > specificity("a"));
    assert_eq!(specificity("*"), 0);
    assert_eq!(specificity(":where(#a)"), 0);
    assert_eq!(specificity(":is(#a, b)"), specificity("#a"));
    assert_eq!(specificity(":not(.a)"), specificity(".a"));
    assert_eq!(specificity("[a]"), specificity(".a"));
    assert_eq!(specificity(":first-child"), specificity(".a"));
    assert_eq!(specificity("::before"), specificity("a"));
}

const PAGE: &str = r#"<!DOCTYPE html>
<div t=div id=main class="x y" lang=en-US><!-- c -->
  <p t=one class=first>one</p>
  text
  <p t=two title="A b">two <b t=bold>bold</b></p>
  <span t=three></span>
  <a t=link href=x>x</a><a t=nolink>y</a>
  <input t=box checked disabled>
</div>
<p t=outside></p>"#;

#[test]
fn matches_simple_selectors() {
    let page = parse(PAGE);
    let d = &page;
    assert!(selects("p", d, "one"));
    assert!(selects("P", d, "one"));
    assert!(!selects("p", d, "three"));
    assert!(selects("*", d, "three"));
    assert!(selects(".x", d, "div"));
    assert!(selects(".y.x", d, "div"));
    assert!(!selects(".X", d, "div"), "class names are case-sensitive");
    assert!(selects("#main", d, "div"));
    assert!(!selects("#MAIN", d, "div"));
    assert!(selects("[title]", d, "two"));
    assert!(selects("[title='A b']", d, "two"));
    assert!(!selects("[title='a b']", d, "two"));
    assert!(selects("[title='a b' i]", d, "two"));
    assert!(selects("[TITLE~=b]", d, "two"));
    assert!(selects("[lang|=en]", d, "div"));
    assert!(selects("[title^=A]", d, "two"));
    assert!(selects("[title$=b]", d, "two"));
    assert!(selects("[title*=' ']", d, "two"));
    assert!(!selects("[title^='']", d, "two"));
}

#[test]
fn matches_combinators() {
    let page = parse(PAGE);
    let d = &page;
    assert!(selects("div b", d, "bold"));
    assert!(selects("html b", d, "bold"));
    assert!(selects("p > b", d, "bold"));
    assert!(!selects("div > b", d, "bold"));
    assert!(
        selects(".first + p", d, "two"),
        "text between does not count"
    );
    assert!(!selects(".first + span", d, "three"));
    assert!(selects(".first ~ span", d, "three"));
    assert!(!selects("span ~ .first", d, "one"));
    assert!(selects("div > p + span", d, "three"));
    assert!(selects("span, b", d, "bold"));
    assert!(selects("html > body div p > b", d, "bold"));
    assert!(!selects("a b", d, "bold"));
}

#[test]
fn matches_pseudo_classes() {
    let page = parse(PAGE);
    let d = &page;
    assert!(
        selects("p:first-child", d, "one"),
        "comments and text do not count"
    );
    assert!(!selects("p:first-child", d, "two"));
    assert!(selects(":last-child", d, "box"));
    assert!(selects("b:only-child", d, "bold"));
    assert!(selects("p:first-of-type", d, "one"));
    assert!(selects("p:last-of-type", d, "two"));
    assert!(selects("span:only-of-type", d, "three"));
    assert!(selects(":nth-child(2)", d, "two"));
    assert!(selects(":nth-child(2n)", d, "two"));
    assert!(selects(":nth-child(odd)", d, "one"));
    assert!(selects(":nth-child(-n+2)", d, "two"));
    assert!(!selects(":nth-child(-n+1)", d, "two"));
    assert!(selects(":nth-of-type(2)", d, "two"));
    assert!(selects(":nth-last-child(1)", d, "box"));
    assert!(selects("span:empty", d, "three"));
    assert!(!selects("p:empty", d, "two"));
    assert!(selects(":link", d, "link"));
    assert!(!selects(":link", d, "nolink"));
    assert!(selects(":checked", d, "box"));
    assert!(selects(":disabled", d, "box"));
    assert!(!selects(":enabled", d, "box"));
    assert!(!selects(":hover", d, "box"));
    assert!(selects(":not(span)", d, "two"));
    assert!(!selects(":not(p, span)", d, "two"));
    assert!(selects(":is(span, p)", d, "two"));
    assert!(selects(":where(div p)", d, "two"));
    let html = page.first_child(page.root()).and_then(|_| {
        page.descendants(page.root())
            .find(|&id| page.tag(id) == Tag::Html)
    });
    let stylesheet = sheet(":root { top: 1 }");
    assert!(matches(d, html.unwrap(), &stylesheet.rules[0].selectors[0]));
}

#[test]
fn quirks_mode_ignores_the_case_of_ids_and_classes() {
    let page = parse("<p t=a class=x id=y>");
    assert!(page.is_quirks());
    assert!(selects(".X", &page, "a"));
    assert!(selects("#Y", &page, "a"));
}

#[test]
fn escapes_are_resolved() {
    let page = parse("<!DOCTYPE html><p t=a class='a:b' id=1x title='\u{17c}'>");
    assert!(selects(".a\\:b", &page, "a"));
    assert!(selects("#\\31 x", &page, "a"));
    assert!(selects("[title='\\17c']", &page, "a"));
}

/// Quadratic or exponential behavior would make these take minutes or never end.
#[test]
fn hostile_input_stays_fast() {
    for source in [
        "{".repeat(200_000),
        "(".repeat(200_000),
        "a{".repeat(100_000),
        "@media x{".repeat(30_000),
        "/*".repeat(100_000),
        "\"".repeat(200_001),
        "url(".repeat(50_000),
        "\\".repeat(200_000),
        "a,".repeat(100_000),
        ":not(".repeat(40_000),
        format!("a{{{}}}", "b:c;".repeat(50_000)),
        format!("{}{{top:1}}", "a ".repeat(50_000)),
    ] {
        let _ = Stylesheet::parse(&source);
    }
    // A long chain of descendant combinators against a deep tree must not backtrack
    // exponentially.
    let page = parse(&format!("<!DOCTYPE html>{}<i t=x>", "<div>".repeat(400)));
    let selector = format!("{} p i", "div ".repeat(28));
    assert!(!selects(&selector, &page, "x"));
}
