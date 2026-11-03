//! Tree construction: expected trees, independence from how the input is split, tree
//! integrity on random and hostile input, and an optional check on real pages.

use std::fmt::Write;

use ron::html::{Document, NodeId, NodeKind, Parser, Tag, parse};

/// Shows a subtree as `name(child,child)`, leaving out attributes.
fn show(document: &Document, id: NodeId, out: &mut String) {
    match document.kind(id) {
        NodeKind::Element => {
            out.push_str(document.name(id));
            out.push('(');
            show_children(document, id, out);
            out.push(')');
        }
        NodeKind::Document | NodeKind::Fragment => show_children(document, id, out),
        NodeKind::Doctype | NodeKind::Text | NodeKind::Comment => {
            write!(out, "{:?}", document.text(id)).unwrap();
        }
    }
}

fn show_children(document: &Document, id: NodeId, out: &mut String) {
    for (index, child) in document.children(id).enumerate() {
        if index > 0 {
            out.push(',');
        }
        show(document, child, out);
    }
}

fn tree(document: &Document) -> String {
    let mut out = String::new();
    show(document, document.root(), &mut out);
    out
}

fn body(inner: &str) -> String {
    format!("html(head(),body({inner}))")
}

/// Parses in pieces of at most `size` bytes, cut at character boundaries.
fn parse_in_pieces(source: &str, mut size: impl FnMut() -> usize) -> Parser {
    let mut parser = Parser::new();
    let mut rest = source;
    while !rest.is_empty() {
        let mut end = size().clamp(1, rest.len());
        while !rest.is_char_boundary(end) {
            end += 1;
        }
        let (piece, tail) = rest.split_at(end);
        parser.feed(piece);
        rest = tail;
    }
    parser.finish();
    parser
}

/// Every link must agree with the links in the opposite direction.
fn assert_links_agree(document: &Document) {
    for id in document.descendants(document.root()) {
        let mut last = None;
        for child in document.children(id) {
            assert_eq!(document.parent(child), Some(id));
            last = Some(child);
        }
        assert_eq!(document.last_child(id), last);
    }
}

fn max_depth(document: &Document) -> usize {
    let root = document.root();
    document
        .descendants(root)
        .map(|id| {
            std::iter::successors(document.parent(id), |&parent| document.parent(parent)).count()
        })
        .max()
        .unwrap_or(0)
}

const CASES: &[(&str, &str)] = &[
    ("<p>a<p>b", r#"p("a"),p("b")"#),
    ("x", r#""x""#),
    ("", ""),
    // The two examples of misnested formatting from the HTML standard.
    (
        "<p>1<b>2<i>3</b>4</i>5</p>",
        r#"p("1",b("2",i("3")),i("4"),"5")"#,
    ),
    ("<b>1<p>2</b>3</p>", r#"b("1"),p(b("2"),"3")"#),
    (
        "<a>1<div>2<a>3</a>4</div>",
        r#"a("1"),div(a("2"),a("3"),"4")"#,
    ),
    (
        "<ol><li><a href=y>Yahoo\n<li><a href=z>Alta</ol>x",
        r#"ol(li(a("Yahoo\n")),li(a("Alta"))),a("x")"#,
    ),
    ("<br><hr><img src=x>a", r#"br(),hr(),img(),"a""#),
    ("<h1>a<h2>b", r#"h1("a"),h2("b")"#),
    (
        "<table><tr><td>a<td>b<tr><td>c</table>d",
        r#"table(tbody(tr(td("a"),td("b")),tr(td("c")))),"d""#,
    ),
    (
        "<table><thead><tr><th>a<tbody><tr><td><table><tr><td>b</table>c</table>",
        r#"table(thead(tr(th("a"))),tbody(tr(td(table(tbody(tr(td("b")))),"c"))))"#,
    ),
    ("<td>a</td><tr>b", r#""a","b""#),
    (
        "<table><col><caption>c</caption></table>",
        r#"table(colgroup(col()),caption("c"))"#,
    ),
    (
        "<ul><li>a<ul><li>b</ul><li>c</ul>",
        r#"ul(li("a",ul(li("b"))),li("c"))"#,
    ),
    ("<div><p>a</div>b", r#"div(p("a")),"b""#),
    ("</p>", ""),
    ("<body></p>", "p()"),
    ("</div>x", r#""x""#),
    ("</br>x", r#"br(),"x""#),
    ("<dl><dt>a<dd>b<dt>c</dl>", r#"dl(dt("a"),dd("b"),dt("c"))"#),
    ("<form><form>a</form>b", r#"form("a"),"b""#),
    ("<button>a<button>b", r#"button("a"),button("b")"#),
    (
        "<select><option>a<option>b</select>c",
        r#"select(option("a"),option("b")),"c""#,
    ),
    (
        "<pre>\nx</pre><textarea>\r\ny</textarea>",
        r#"pre("x"),textarea("y")"#,
    ),
    ("<pre>\rx</pre><pre>\r\r</pre>", r#"pre("x"),pre("\r")"#),
    (
        "<svg><path/><circle/></svg>x",
        r#"svg(path(),circle()),"x""#,
    ),
    ("<br/>x<div/>y", r#"br(),"x",div("y")"#),
    ("<body><p>a</body></html>  ", r#"p("a","  ")"#),
    ("<body>a</body>b", r#""a","b""#),
    ("a < b", r#""a < b""#),
    (
        "<p><b><b><b><b>a</p><p>x",
        r#"p(b(b(b(b("a"))))),p(b(b(b("x"))))"#,
    ),
    (
        "<custom-x>a</custom-x>b<CUSTOM>c</custom>",
        r#"custom-x("a"),"b",CUSTOM("c")"#,
    ),
    ("<p><span>a</p>b", r#"p(span("a")),"b""#),
    ("<nobr>a<nobr>b", r#"nobr("a"),nobr("b")"#),
    ("<object><b>a</object>c", r#"object(b("a")),"c""#),
    // Misplaced table content goes in front of the table.
    (
        "<table>x<tr><td>y</table>",
        r#""x",table(tbody(tr(td("y"))))"#,
    ),
    ("<table> <tr> </table>", r#"table(" ",tbody(tr(" ")))"#),
    // Foreign content has its own rules: no raw text, self-closing tags, CDATA.
    ("<svg><title><p>x</title></svg>", r#"svg(title(p("x")))"#),
    ("<svg><![CDATA[<a>]]></svg>", r#"svg("<a>")"#),
    ("<svg><p>x", r#"svg(),p("x")"#),
    // A tag cut off by the end of the input does not count.
    ("x<di", r#""x""#),
    ("x<a href=\"y>z", r#""x""#),
    ("<select><option>a<select>b", r#"select(option("a")),"b""#),
];

const WHOLE_DOCUMENTS: &[(&str, &str)] = &[
    (
        "<html><head><title>x</title></head><body><title>y</title>z",
        r#"html(head(title("x")),body(title("y"),"z"))"#,
    ),
    (
        "<HTML>\r\n<HEAD>\r\n<TITLE>t</TITLE>\r\n</HEAD>\r\n<body>\r\n<BR>\r\n</body>\r\n</HTML>",
        r#"html(head("\r\n",title("t"),"\r\n"),"\r\n",body("\r\n",br(),"\r\n","\r\n"))"#,
    ),
    (
        "<!-- c --><!DOCTYPE html><p>x",
        r#""<!-- c -->","<!DOCTYPE html>",html(head(),body(p("x")))"#,
    ),
    (
        "<script>a<b</script><style>c</style>d",
        r#"html(head(script("a<b"),style("c")),body("d"))"#,
    ),
    (
        "<head></head><title>late</title>x",
        r#"html(head(title("late")),body("x"))"#,
    ),
    (
        "<frameset><frame></frameset>",
        "html(head(),frameset(frame()))",
    ),
    (
        "<template><tr><td>x</template>",
        r#"html(head(template(tr(td("x")))),body())"#,
    ),
    (
        "<script><!--<script></script>x</script>y",
        r#"html(head(script("<!--<script></script>x")),body("y"))"#,
    ),
];

#[test]
fn builds_the_expected_trees() {
    for (source, expected) in CASES {
        assert_eq!(tree(&parse(source)), body(expected), "{source:?}");
    }
    for (source, expected) in WHOLE_DOCUMENTS {
        assert_eq!(tree(&parse(source)), *expected, "{source:?}");
    }
}

#[test]
fn every_split_gives_the_same_tree() {
    let sources = CASES
        .iter()
        .chain(WHOLE_DOCUMENTS)
        .map(|(source, _)| *source);
    for source in sources {
        let whole = tree(&parse(source));
        for size in 1..=source.len() {
            let parser = parse_in_pieces(source, || size);
            assert_eq!(tree(parser.document()), whole, "{source:?} by {size}");
        }
    }
}

#[test]
fn the_tree_can_be_read_before_the_end() {
    let mut parser = Parser::new();
    parser.feed("<title>T</title><p>Hel");
    assert_eq!(
        tree(parser.document()),
        r#"html(head(title("T")),body(p("Hel")))"#
    );
    assert_eq!(parser.document().title(), Some("T"));
    parser.feed("lo<b");
    assert_eq!(
        tree(parser.document()),
        r#"html(head(title("T")),body(p("Hello")))"#
    );
    parser.feed(">w");
    assert_eq!(
        tree(parser.document()),
        r#"html(head(title("T")),body(p("Hello",b("w"))))"#
    );
    parser.finish();
    assert!(!parser.is_truncated());
}

#[test]
fn finds_the_first_title() {
    assert_eq!(
        parse("<title>a</title><body><title>b</title>").title(),
        Some("a")
    );
    assert_eq!(parse("<title></title>").title(), Some(""));
    assert_eq!(parse("<p>x").title(), None);
}

#[test]
fn elements_read_attributes_from_the_source() {
    let document = parse("<IMG SRC=a.gif align=right><p>x");
    let image = document
        .descendants(document.root())
        .find(|&id| document.tag(id) == Tag::Img)
        .unwrap();
    assert_eq!(document.name(image), "img");
    let attributes: Vec<_> = document
        .attributes(image)
        .map(|attribute| (attribute.name, attribute.value))
        .collect();
    assert_eq!(
        attributes,
        [("SRC", Some("a.gif")), ("align", Some("right"))]
    );

    let html = document.first_child(document.root()).unwrap();
    assert_eq!(document.attributes(html).count(), 0, "implied element");
}

#[test]
fn character_references_are_decoded_when_read() {
    let document = parse(
        "<p title=\"a&amp;b&copy=c\">&#169; &copy; &amp &#x80; &nosuch; &#0;</p><title>&lt;</title><style>&lt;</style>\r\n",
    );
    let find = |tag: Tag| {
        document
            .descendants(document.root())
            .find(|&id| document.tag(id) == tag)
            .unwrap()
    };
    let paragraph = find(Tag::P);
    let text = document.first_child(paragraph).unwrap();
    assert_eq!(
        document.decoded_text(text),
        "\u{a9} \u{a9} & \u{20ac} &nosuch; \u{fffd}"
    );
    let attribute = document.attributes(paragraph).next().unwrap();
    assert_eq!(attribute.decoded_value(), "a&b&copy=c");
    let title = document.first_child(find(Tag::Title)).unwrap();
    assert_eq!(document.decoded_text(title), "<");
    let style = document.first_child(find(Tag::Style)).unwrap();
    assert_eq!(document.decoded_text(style), "&lt;");
}

#[test]
fn reopened_formatting_keeps_its_attributes() {
    let document = parse("<p><font size=1>a</p><p>b");
    let fonts: Vec<_> = document
        .descendants(document.root())
        .filter(|&id| document.tag(id) == Tag::Font)
        .map(|id| document.attributes(id).next().unwrap().value)
        .collect();
    assert_eq!(fonts, [Some("1"), Some("1")]);
}

#[test]
fn random_markup_gives_a_sound_tree_however_it_is_split() {
    const ATOMS: [&str; 62] = [
        "<",
        ">",
        "/",
        "=",
        "\"",
        " ",
        "\n",
        "a",
        "ż",
        "<p>",
        "</p>",
        "<b>",
        "</b>",
        "<i>",
        "</i>",
        "<a>",
        "</a>",
        "<div>",
        "</div>",
        "<table>",
        "</table>",
        "<tr>",
        "</tr>",
        "<td>",
        "</td>",
        "<li>",
        "</li>",
        "<ul>",
        "</ul>",
        "<body>",
        "</body>",
        "<html>",
        "</html>",
        "<head>",
        "</head>",
        "<title>",
        "</title>",
        "<script>",
        "</script>",
        "<!--",
        "-->",
        "<form>",
        "</form>",
        "<select>",
        "<option>",
        "<svg>",
        "</svg>",
        "<br/>",
        "</br>",
        "<h1>",
        "</h2>",
        "<nobr>",
        "<object>",
        "</object>",
        "<button>",
        "<caption>",
        "<col>",
        "<pre>",
        "<x>",
        "</x>",
        "<font size=1>",
        "<!DOCTYPE x>",
    ];
    let mut seed = 77_u32;
    let mut next = move |limit: usize| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 16) as usize % limit
    };
    for _ in 0..2000 {
        let count = next(120);
        let source: String = (0..count).map(|_| ATOMS[next(ATOMS.len())]).collect();
        let whole = parse(&source);
        assert_links_agree(&whole);
        let parser = parse_in_pieces(&source, || next(9));
        assert_eq!(tree(parser.document()), tree(&whole), "{source:?}");
    }
}

/// Searches walk the open elements, so these would take minutes if depth were not bounded.
#[test]
fn hostile_repetitions_stay_bounded() {
    for unit in [
        "<div>",
        "<b>",
        "<a>",
        "<p>",
        "<p><b>",
        "<table>",
        "<li>",
        "</b>",
        "<b><p>",
        "<a><div>",
        "<table><tr><td>",
        "<b><i><u><em><p>x</p>",
        "<div></b>",
        "<b><div></b>x",
        "<a><table>",
    ] {
        let document = parse(&unit.repeat(200_000 / unit.len()));
        // The limit applies to open elements. Repairing formatting takes elements off
        // the stack but not out of the tree, so `<a><table>` reaches about twice the limit.
        assert!(max_depth(&document) <= 1100, "{unit:?}");
        assert_links_agree(&document);
    }
}

#[test]
fn clone_budget_stops_hostile_growth_but_keeps_the_text() {
    let source: String = (0..30_000).map(|i| format!("<p><b a={i}>x")).collect();
    let parser = parse_in_pieces(&source, || 1400);
    assert!(parser.is_formatting_limited());
    let document = parser.document();
    let bytes = document.source_capacity_bytes() + document.nodes_capacity_bytes();
    assert!(
        bytes <= 20 * source.len(),
        "{bytes} bytes for {}",
        source.len()
    );
    let texts = document
        .descendants(document.root())
        .filter(|&id| document.kind(id) == NodeKind::Text)
        .count();
    assert_eq!(texts, 30_000);
    assert_links_agree(document);
    assert_eq!(tree(document), tree(&parse(&source)), "split independence");
}

#[test]
fn ordinary_unclosed_formatting_stays_within_the_budget() {
    let source = "<p><font size=2><b><i>a short line of text</p>".repeat(5000);
    let mut parser = Parser::new();
    parser.feed(&source);
    parser.finish();
    assert!(!parser.is_formatting_limited());
}

/// Checks real pages when `RON_CORPUS_DIR` names a directory of UTF-8 `.html` files:
/// `RON_CORPUS_DIR=... cargo test --release -- --ignored`
#[test]
#[ignore = "needs RON_CORPUS_DIR and takes seconds per page"]
fn real_pages_stay_small_and_independent_of_splitting() {
    let directory = std::env::var("RON_CORPUS_DIR").expect("RON_CORPUS_DIR must be set");
    let mut checked = 0;
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "html") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let whole = parse(&source);
        assert_links_agree(&whole);
        let mut parser = Parser::new();
        parser.feed(&source);
        parser.finish();
        assert!(!parser.is_formatting_limited(), "{path:?}");
        let bytes = whole.source_capacity_bytes() + whole.nodes_capacity_bytes();
        assert!(bytes <= 3 * source.len(), "{path:?} takes {bytes} bytes");
        for size in [1, 1400, 65536] {
            let parser = parse_in_pieces(&source, || size);
            let document = parser.document();
            let mut nodes = document.descendants(document.root());
            for id in whole.descendants(whole.root()) {
                let other = nodes.next().unwrap();
                assert_eq!(whole.kind(id), document.kind(other), "{path:?} by {size}");
                assert_eq!(whole.text(id), document.text(other), "{path:?} by {size}");
            }
            assert!(nodes.next().is_none(), "{path:?} by {size}");
        }
        checked += 1;
    }
    assert!(checked > 0, "no .html files found");
}
