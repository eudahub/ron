//! The cascade and computed styles.

use ron::css::{
    BorderStyle, Color, ComputedStyle, Display, Float, FontStyle, ListStyleType, Size, Styles,
    Stylesheet, TextAlign, UNDERLINE, VerticalAlign, VerticalPosition, Viewport, WhiteSpace,
    compute_styles, document_stylesheets,
};
use ron::html::{Document, NodeId, parse};

const RED: Color = Color::rgb(255, 0, 0);
const GREEN: Color = Color::rgb(0, 128, 0);
const BLUE: Color = Color::rgb(0, 0, 255);

struct Page {
    document: Document,
    styles: Styles,
}

impl Page {
    fn new(source: &str) -> Self {
        Self::sized(source, Viewport::default())
    }

    fn sized(source: &str, viewport: Viewport) -> Self {
        let document = parse(source);
        let sheets = document_stylesheets(&document, viewport);
        let styles = compute_styles(&document, &sheets, viewport);
        Self { document, styles }
    }

    fn element(&self, wanted: &str) -> NodeId {
        self.document
            .descendants(self.document.root())
            .find(|&id| {
                self.document
                    .attributes(id)
                    .any(|attribute| attribute.name == "t" && attribute.value == Some(wanted))
            })
            .expect("element with this t attribute")
    }

    /// The style of the element marked `t=wanted`.
    fn style(&self, wanted: &str) -> &ComputedStyle {
        self.styles.get(self.element(wanted))
    }
}

/// A page with the given style sheet and one `<p t=a>` inside `<div t=d>`.
fn styled(css: &str) -> Page {
    Page::new(&format!(
        "<!DOCTYPE html><style>{css}</style><div t=d id=i class=c><p t=a id=j class=k>x</p></div>"
    ))
}

#[test]
fn defaults_follow_the_standard() {
    let page = Page::new(
        "<!DOCTYPE html><title t=title>x</title><h1 t=h1>a</h1><p t=p>b <b t=b>c</b> <i t=i>d</i>\
         <a t=a href=x>e</a><a t=noa>f</a></p><ul t=ul><li t=li>g</ul><ol t=ol><li>h</ol>\
         <table t=table><tr t=tr><td t=td>i<th t=th>j</table><pre t=pre>k</pre><hr t=hr>",
    );
    assert_eq!(page.style("title").display, Display::None);
    assert_eq!(page.style("h1").display, Display::Block);
    assert!((page.style("h1").font_size - 32.0).abs() < 0.01);
    assert_eq!(page.style("h1").font_weight, 700);
    assert_eq!(page.style("h1").margin.top, Size::Px(0.67 * 32.0));
    assert_eq!(page.style("p").margin.top, Size::Px(16.0));
    assert_eq!(page.style("b").font_weight, 700);
    assert_eq!(page.style("b").display, Display::Inline);
    assert_eq!(page.style("i").font_style, FontStyle::Italic);
    assert_eq!(page.style("a").color, Color::rgb(0, 0, 0xee));
    assert_eq!(page.style("a").text_decoration, UNDERLINE);
    assert_eq!(page.style("noa").color, Color::BLACK);
    assert_eq!(page.style("ul").list_style_type, ListStyleType::Disc);
    assert_eq!(page.style("ul").padding.left, Size::Px(40.0));
    assert_eq!(page.style("li").display, Display::ListItem);
    assert_eq!(page.style("ol").list_style_type, ListStyleType::Decimal);
    assert_eq!(page.style("table").display, Display::Table);
    assert_eq!(page.style("tr").display, Display::TableRow);
    assert_eq!(page.style("td").display, Display::TableCell);
    assert_eq!(page.style("th").text_align, TextAlign::Center);
    assert_eq!(page.style("pre").white_space, WhiteSpace::Pre);
    assert_eq!(page.style("pre").font_family.as_ref(), "monospace");
    assert_eq!(page.style("hr").border_style.top, BorderStyle::Inset);
    assert_eq!(page.style("hr").border_width.top, 1.0);
}

#[test]
fn later_and_more_specific_rules_win() {
    let color = |css: &str| styled(css).style("a").color;
    assert_eq!(color("p { color: red } p { color: blue }"), BLUE);
    assert_eq!(color(".k { color: blue } p { color: red }"), BLUE);
    assert_eq!(color("#j { color: blue } .k.k.k { color: red }"), BLUE);
    assert_eq!(color("p { color: red !important } #j { color: blue }"), RED);
    assert_eq!(
        color("#j { color: red !important } p { color: blue !important }"),
        RED
    );
    assert_eq!(color("p { color: red; color: blue }"), BLUE);
    assert_eq!(color("p { color: red; color: nonsense }"), RED);
    assert_eq!(color("p, #none { color: red } div p { color: blue }"), BLUE);
}

#[test]
fn style_attribute_beats_rules_except_important_ones() {
    let page = |css: &str, inline: &str| {
        Page::new(&format!(
            "<!DOCTYPE html><style>{css}</style><p t=a id=j style='{inline}'>x"
        ))
    };
    assert_eq!(
        page("#j { color: red }", "color: blue").style("a").color,
        BLUE
    );
    assert_eq!(
        page("#j { color: red !important }", "color: blue")
            .style("a")
            .color,
        RED
    );
    assert_eq!(
        page("#j { color: red !important }", "color: blue !important")
            .style("a")
            .color,
        BLUE
    );
    assert_eq!(
        page("", "color: blue; margin: nonsense").style("a").color,
        BLUE
    );
}

#[test]
fn authors_override_defaults_unless_defaults_are_important() {
    assert_eq!(
        styled("p { display: inline }").style("a").display,
        Display::Inline
    );
    assert_eq!(
        styled("p { margin-top: 0 }").style("a").margin.top,
        Size::Px(0.0)
    );
}

#[test]
fn inheritance_passes_only_inherited_properties() {
    let page =
        styled("div { color: red; margin-top: 7px; font-size: 20px; background-color: blue }");
    assert_eq!(page.style("a").color, RED);
    assert_eq!(page.style("a").font_size, 20.0);
    assert_eq!(page.style("a").background_color, Color::TRANSPARENT);
    let page = styled(
        "div { margin-left: 7px; background-color: blue; color: red } p { margin-left: inherit; background-color: inherit; color: initial }",
    );
    assert_eq!(page.style("a").margin.left, Size::Px(7.0));
    assert_eq!(page.style("a").background_color, BLUE);
    assert_eq!(page.style("a").color, Color::BLACK);
}

#[test]
fn lengths_resolve_against_font_sizes_and_the_view() {
    let page = styled(
        "html { font-size: 10px } div { font-size: 200% } p { font-size: 1.5em; margin-top: 2em; margin-left: 1rem; width: 50%; height: 10vh; padding-top: 1in; padding-left: 12pt }",
    );
    let style = page.style("a");
    assert_eq!(page.style("d").font_size, 20.0);
    assert_eq!(style.font_size, 30.0);
    assert_eq!(style.margin.top, Size::Px(60.0));
    assert_eq!(style.margin.left, Size::Px(10.0));
    assert_eq!(style.width, Size::Percent(50.0));
    assert_eq!(style.height, Size::Px(76.8));
    assert_eq!(style.padding.top, Size::Px(96.0));
    assert_eq!(style.padding.left, Size::Px(16.0));
    assert_eq!(
        styled("p { margin-top: 5 }").style("a").margin.top,
        Size::Px(16.0),
        "a unit is required"
    );
    assert_eq!(
        styled("p { padding-top: -1px }").style("a").padding.top,
        Size::Px(0.0),
        "cannot be negative"
    );
    assert!((styled("div { font-size: smaller }").style("d").font_size - 16.0 / 1.2).abs() < 0.01);
    assert_eq!(
        styled("div { font-size: x-large }").style("d").font_size,
        24.0
    );
}

#[test]
fn colors_come_in_every_form() {
    let color = |value: &str| styled(&format!("p {{ color: {value} }}")).style("a").color;
    assert_eq!(color("red"), RED);
    assert_eq!(color("RED"), RED);
    assert_eq!(color("#f00"), RED);
    assert_eq!(color("#ff0000"), RED);
    assert_eq!(color("rgb(255, 0, 0)"), RED);
    assert_eq!(color("rgb(255 0 0)"), RED);
    assert_eq!(color("rgb(100%, 0%, 0%)"), RED);
    assert_eq!(color("hsl(0, 100%, 50%)"), RED);
    assert_eq!(color("rebeccapurple"), Color::rgb(0x66, 0x33, 0x99));
    assert_eq!(color("rgba(255, 0, 0, 0.5)").alpha, 128);
    assert_eq!(color("#f008").alpha, 0x88);
    assert_eq!(color("transparent"), Color::TRANSPARENT);
    assert_eq!(color("#ggg"), Color::BLACK, "invalid values are dropped");
    assert_eq!(
        styled("p { color: red; border-top-color: currentcolor; border-top-style: solid }")
            .style("a")
            .border_color
            .top,
        RED
    );
    assert_eq!(
        styled("div { color: green } p { color: currentcolor }")
            .style("a")
            .color,
        GREEN
    );
}

#[test]
fn shorthands_expand() {
    let page = styled(
        "p { margin: 1px 2px 3px 4px; padding: 5px 6px; border: 2px solid red; background: url(a.png) blue no-repeat; font: italic bold 20px/30px Arial, serif; list-style: square; text-decoration: underline red }",
    );
    let style = page.style("a");
    assert_eq!(style.margin.top, Size::Px(1.0));
    assert_eq!(style.margin.right, Size::Px(2.0));
    assert_eq!(style.margin.bottom, Size::Px(3.0));
    assert_eq!(style.margin.left, Size::Px(4.0));
    assert_eq!(style.padding.top, Size::Px(5.0));
    assert_eq!(style.padding.left, Size::Px(6.0));
    assert_eq!(style.border_width.left, 2.0);
    assert_eq!(style.border_style.bottom, BorderStyle::Solid);
    assert_eq!(style.border_color.right, RED);
    assert_eq!(style.background_color, BLUE);
    assert_eq!(style.background_image.as_deref(), Some("a.png"));
    assert_eq!(style.font_style, FontStyle::Italic);
    assert_eq!(style.font_weight, 700);
    assert_eq!(style.font_size, 20.0);
    assert_eq!(style.font_family.as_ref(), "Arial, serif");
    assert_eq!(style.list_style_type, ListStyleType::Square);
    assert_eq!(style.text_decoration, UNDERLINE);
    let auto = styled("p { margin: 0 auto }");
    assert_eq!(auto.style("a").margin.left, Size::Auto);
    assert_eq!(
        styled("p { margin: 1px 2px 3px 4px 5px }")
            .style("a")
            .margin
            .left,
        Size::Px(0.0)
    );
    assert_eq!(
        styled("p { border: solid }").style("a").border_width.left,
        3.0
    );
    assert_eq!(
        styled("p { border: 2px }").style("a").border_width.left,
        0.0,
        "no style, no border"
    );
}

#[test]
fn values_it_cannot_read_are_dropped() {
    assert_eq!(
        styled("p { color: red; color: var(--x) }").style("a").color,
        RED
    );
    assert_eq!(
        styled("p { width: 5px; width: calc(1px + 1px) }")
            .style("a")
            .width,
        Size::Px(5.0)
    );
    assert_eq!(
        styled("p { nonsense: 1; color: red }").style("a").color,
        RED
    );
}

#[test]
fn floats_and_roots_become_blocks() {
    assert_eq!(
        styled("div { display: inline; float: left }")
            .style("d")
            .display,
        Display::Block
    );
    assert_eq!(
        styled("div { display: inline; position: absolute }")
            .style("d")
            .display,
        Display::Block
    );
    let root = Page::new("<!DOCTYPE html><html t=h style='display:inline'>");
    assert_eq!(root.style("h").display, Display::Block);
}

#[test]
fn media_conditions_are_checked_with_all_around_them() {
    let small = Viewport {
        width: 500.0,
        height: 800.0,
    };
    let page = |css: &str, viewport| {
        Page::sized(
            &format!("<!DOCTYPE html><style>{css}</style><p t=a>x"),
            viewport,
        )
        .style("a")
        .color
    };
    let big = Viewport::default();
    assert_eq!(
        page("@media (min-width: 600px) { p { color: red } }", big),
        RED
    );
    assert_eq!(
        page("@media (min-width: 600px) { p { color: red } }", small),
        Color::BLACK
    );
    assert_eq!(
        page("@media (max-width: 600px) { p { color: red } }", small),
        RED
    );
    assert_eq!(page("@media print { p { color: red } }", big), Color::BLACK);
    assert_eq!(page("@media screen { p { color: red } }", big), RED);
    assert_eq!(page("@media not print { p { color: red } }", big), RED);
    assert_eq!(page("@media print, screen { p { color: red } }", big), RED);
    assert_eq!(
        page(
            "@media screen and (orientation: portrait) { p { color: red } }",
            small
        ),
        RED
    );
    assert_eq!(
        page("@media (unknown: 1) { p { color: red } }", big),
        Color::BLACK
    );
    assert_eq!(
        page("@media screen { @media print { p { color: red } } }", big),
        Color::BLACK
    );
    assert_eq!(
        page("@media print { @media screen { p { color: red } } }", big),
        Color::BLACK
    );
    let attribute = Page::new("<!DOCTYPE html><style media=print>p { color: red }</style><p t=a>");
    assert_eq!(attribute.style("a").color, Color::BLACK);
}

#[test]
fn old_attributes_give_styles_that_css_can_override() {
    let page = Page::new(
        "<body t=b bgcolor=red text='#00ff00' background=paper.gif>\
         <img t=img src=x align=right border=2 width=50 height=60>\
         <hr t=hr width=60%>\
         <font t=small size=-1>a</font><font t=big size=5 color=blue face=Arial>b</font>\
         <p t=p align=center>c</p>\
         <ol t=ol type=A><li>d</ol>\
         <table t=table border=1 cellpadding=5 cellspacing=3 width=100 bgcolor=fff align=center>\
         <tr t=tr valign=top><td t=td nowrap align=right>e</table>",
    );
    assert_eq!(page.style("b").background_color, RED);
    assert_eq!(page.style("b").color, Color::rgb(0, 255, 0));
    assert_eq!(
        page.style("b").background_image.as_deref(),
        Some("paper.gif")
    );
    assert_eq!(page.style("img").float, Float::Right);
    assert_eq!(page.style("img").display, Display::Block);
    assert_eq!(page.style("img").border_width.top, 2.0);
    assert_eq!(page.style("img").border_style.top, BorderStyle::Solid);
    assert_eq!(page.style("img").width, Size::Px(50.0));
    assert_eq!(page.style("img").height, Size::Px(60.0));
    assert_eq!(page.style("hr").width, Size::Percent(60.0));
    assert_eq!(page.style("small").font_size, 13.0);
    assert_eq!(page.style("big").font_size, 24.0);
    assert_eq!(page.style("big").color, BLUE);
    assert_eq!(page.style("big").font_family.as_ref(), "Arial");
    assert_eq!(page.style("p").text_align, TextAlign::Center);
    assert_eq!(page.style("ol").list_style_type, ListStyleType::UpperAlpha);
    assert_eq!(page.style("table").width, Size::Px(100.0));
    assert_eq!(page.style("table").border_spacing, 3.0);
    assert_eq!(
        page.style("table").background_color,
        Color::rgb(255, 255, 255)
    );
    assert_eq!(page.style("table").margin.left, Size::Auto);
    assert_eq!(page.style("td").padding.top, Size::Px(5.0));
    assert_eq!(page.style("td").border_width.top, 1.0);
    assert_eq!(page.style("td").white_space, WhiteSpace::Nowrap);
    assert_eq!(page.style("td").text_align, TextAlign::Right);
    assert_eq!(
        page.style("tr").vertical_align,
        VerticalPosition::Keyword(VerticalAlign::Top)
    );

    let overridden =
        Page::new("<!DOCTYPE html><style>p { text-align: left }</style><p t=p align=center>");
    assert_eq!(overridden.style("p").text_align, TextAlign::Left);
}

#[test]
fn elements_alike_share_one_style() {
    let source = format!(
        "<!DOCTYPE html><ul>{}</ul>",
        "<li>item<b>x</b>".repeat(1000)
    );
    let page = Page::new(&source);
    assert!(
        page.styles.distinct_count() < 12,
        "{} styles",
        page.styles.distinct_count()
    );
    let styled_source = format!(
        "<!DOCTYPE html>{}",
        (0..100)
            .map(|i| format!("<p style='margin-top:{i}px'>"))
            .collect::<String>()
    );
    let page = Page::new(&styled_source);
    assert!(
        page.styles.distinct_count() > 100,
        "each style attribute differs"
    );
}

#[test]
fn text_nodes_read_their_parents_style() {
    let page = styled("p { color: red }");
    let text = page.document.first_child(page.element("a")).unwrap();
    assert_eq!(page.styles.get(text).color, RED);
}

/// Without bounds these would take hours or gigabytes.
#[test]
fn hostile_pages_stay_bounded() {
    let many = format!(
        "<!DOCTYPE html><style>p:nth-child(2n) {{ color: red }} p:nth-of-type(3) {{ color: blue }}\
         .x ~ p {{ color: green }} p ~ p ~ p ~ p ~ .y {{ color: red }} :not(.a) .b {{ top: 1px }}</style>{}",
        "<p>".repeat(5_000)
    );
    let page = Page::new(&many);
    assert!(page.document.node_count() > 5_000);

    let styles: String = (0..50_000)
        .map(|i| format!("<p style=margin:{i}px>"))
        .collect();
    let page = Page::new(&format!("<!DOCTYPE html>{styles}"));
    assert!(page.styles.capacity_bytes() < 30 * styles.len());

    let selectors: String = (0..20_000).map(|i| format!(".c{i},")).collect();
    let rule =
        format!("<!DOCTYPE html><style>{selectors}p {{ color: red }}</style><p t=a class='c5 c6'>");
    assert_eq!(Page::new(&rule).style("a").color, RED);

    let _ = Stylesheet::parse_declarations(&"a:b;".repeat(50_000));
}
