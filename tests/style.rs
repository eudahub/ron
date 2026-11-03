//! The cascade and computed styles.

use ron::css::{
    BackgroundRepeat, BorderStyle, Color, ComputedStyle, Display, Float, FontStyle, LineHeight,
    ListStyleType, Size, Styles, Stylesheet, TextAlign, UNDERLINE, VerticalAlign, VerticalPosition,
    Viewport, WhiteSpace, compute_styles, document_stylesheets,
};
use ron::html::{Document, NodeId, NodeKind, parse};

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
    assert_eq!(page.style("pre").font_size, 12.0);
    assert_eq!(page.style("hr").border_style.top, BorderStyle::Inset);
    assert_eq!(page.style("hr").border_width.top, 1.0);
}

#[test]
fn generic_monospace_is_smaller_as_in_firefox() {
    let page = Page::new(
        "<!DOCTYPE html><pre t=pre>a<code t=inner>b</code></pre><code t=code>c</code>\
         <p style='font-size:20px'><code t=scaled>d</code></p>\
         <code t=own style='font-size:18px'>e</code>\
         <div t=css style='font-family:monospace'>f<span t=back style='font-family:serif'>g</span></div>\
         <div t=named style='font-family:Courier'>h</div>\
         <div t=hack style='font-family:monospace, monospace'>i</div>",
    );
    assert_eq!(page.style("pre").font_size, 12.0);
    assert_eq!(page.style("inner").font_size, 12.0, "not again when nested");
    assert_eq!(page.style("code").font_size, 12.0);
    assert_eq!(page.style("scaled").font_size, 15.0);
    assert_eq!(
        page.style("own").font_size,
        18.0,
        "a size that is set stays"
    );
    assert_eq!(
        page.style("css").font_size,
        12.0,
        "with the family from an author too"
    );
    assert_eq!(page.style("back").font_size, 16.0, "and back again");
    assert_eq!(
        page.style("named").font_size,
        16.0,
        "only the generic family"
    );
    assert_eq!(page.style("hack").font_size, 16.0, "the way pages avoid it");
}

#[test]
fn quirks_mode_adds_its_rules() {
    let quirks = Page::new(
        "<body><h1 t=first>a</h1><p t=second>b</p><font size=1><table t=table><tr><td t=td><p t=cell>c</p></table></font>\
         <img t=right align=right>",
    );
    assert_eq!(quirks.style("first").margin.top, Size::Px(0.0));
    assert_eq!(quirks.style("second").margin.top, Size::Px(16.0));
    assert_eq!(
        quirks.style("table").font_size,
        16.0,
        "tables do not inherit fonts"
    );
    assert_eq!(quirks.style("cell").margin.top, Size::Px(0.0));
    assert_eq!(quirks.style("cell").margin.bottom, Size::Px(0.0));
    assert_eq!(quirks.style("right").margin.left, Size::Px(3.0));

    let standard =
        Page::new("<!DOCTYPE html><h1 t=first>a</h1><font size=1><table t=table></table></font>");
    assert_eq!(standard.style("first").margin.top, Size::Px(0.67 * 32.0));
    assert_eq!(standard.style("table").font_size, 10.0);

    let author = Page::new("<style>h1 { margin-top: 5px }</style><h1 t=first>a</h1>");
    assert_eq!(
        author.style("first").margin.top,
        Size::Px(5.0),
        "authors still decide"
    );
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
fn backgrounds_keep_their_image_repeat_and_position() {
    let style = |css: &str| styled(&format!("p {{ {css} }}"));
    let page = style("background: url(a.png) no-repeat right 5px red");
    let s = page.style("a");
    assert_eq!(s.background_image.as_deref(), Some("a.png"));
    assert_eq!(s.background_repeat, BackgroundRepeat::NoRepeat);
    assert_eq!(s.background_position_x, Size::Percent(100.0));
    assert_eq!(s.background_position_y, Size::Px(5.0));
    assert_eq!(s.background_color, RED);

    let page = style(
        "background-image: url(a.png); background-repeat: repeat-x; background-position: top center",
    );
    let s = page.style("a");
    assert_eq!(s.background_repeat, BackgroundRepeat::RepeatX);
    assert_eq!(s.background_position_x, Size::Percent(50.0));
    assert_eq!(s.background_position_y, Size::Percent(0.0));

    let page = style("background: url(a.png) center / cover no-repeat");
    let s = page.style("a");
    assert_eq!(s.background_position_x, Size::Percent(50.0));
    assert_eq!(s.background_position_y, Size::Percent(50.0));

    let page = style("background-repeat: no-repeat; background: red");
    assert_eq!(
        page.style("a").background_repeat,
        BackgroundRepeat::Repeat,
        "the shorthand resets"
    );
    assert_eq!(page.style("a").background_image, None);
    assert_eq!(
        style("background: nonsense red")
            .style("a")
            .background_color,
        Color::TRANSPARENT
    );
}

fn width_of(value: &str) -> Size {
    styled(&format!(
        "p {{ font-size: 10px; width: 5px; width: {value} }}"
    ))
    .style("a")
    .width
}

#[test]
fn expressions_come_to_pixels_when_they_can() {
    let pairs = [
        ("calc(1px + 2px)", 3.0),
        ("calc(10px - 3px)", 7.0),
        ("calc(2 * 3px)", 6.0),
        ("calc(3px * 2)", 6.0),
        ("calc(9px / 3)", 3.0),
        ("calc((1px + 2px) * (1 + 1))", 6.0),
        ("calc(2em + 1px)", 21.0),
        ("CALC(1PX + 1PX)", 2.0),
        ("calc(calc(1px + 1px) * 2)", 4.0),
        ("min(5px, 3px, 9px)", 3.0),
        ("max(5px, 3px)", 5.0),
        ("clamp(10px, 5px, 20px)", 10.0),
        ("clamp(10px, 15px, 20px)", 15.0),
        ("clamp(10px, 25px, 20px)", 20.0),
        ("calc(min(5px, 3px) + 1px)", 4.0),
        ("calc(0)", 0.0),
    ];
    for (value, wanted) in pairs {
        assert_eq!(width_of(value), Size::Px(wanted), "{value}");
    }
}

#[test]
fn expressions_with_a_percentage_wait_for_layout() {
    assert_eq!(width_of("calc(50%)"), Size::Percent(50.0));
    assert_eq!(width_of("calc(25% * 2)"), Size::Percent(50.0));
    let wide = |value: &str, whole: f32| match width_of(value) {
        Size::Calc(calc) => calc.of(whole),
        other => panic!("{value} gave {other:?}"),
    };
    assert_eq!(wide("calc(100% - 20px)", 200.0), 180.0);
    assert_eq!(wide("calc(50% + 1em)", 200.0), 110.0);
    assert_eq!(wide("min(100%, 960px)", 500.0), 500.0);
    assert_eq!(wide("min(100%, 960px)", 2000.0), 960.0);
    assert_eq!(wide("max(50%, 300px)", 200.0), 300.0);
    assert_eq!(wide("clamp(100px, 50%, 300px)", 100.0), 100.0);
    assert_eq!(wide("clamp(100px, 50%, 300px)", 400.0), 200.0);
    assert_eq!(wide("clamp(100px, 50%, 300px)", 1000.0), 300.0);
    assert_eq!(wide("calc(min(100%, 500px) - 40px)", 1000.0), 460.0);
    assert_eq!(wide("calc(2 * min(50%, 100px))", 1000.0), 200.0);
    // The low bound goes before the high one, when they cross.
    assert_eq!(wide("clamp(300px, 50%, 100px)", 1000.0), 300.0);
}

#[test]
fn expressions_that_make_no_sense_are_dropped() {
    for value in [
        "calc(1px+1px)",
        "calc(1px -1px)",
        "calc(1px + 1)",
        "calc(1px * 1px)",
        "calc(1px / 0)",
        "calc(1px / 1px)",
        "calc()",
        "calc(1px",
        "calc(1px))",
        "min()",
        "clamp(1px, 2px)",
        "calc(1foo + 1px)",
        "unknown(1px)",
        "calc(1e39px + 1e39px)",
        "calc(min(10%, 20%, 30%))",
    ] {
        assert_eq!(width_of(value), Size::Px(5.0), "{value}");
    }
    let deep = format!("{}1px{}", "calc(".repeat(1000), ")".repeat(1000));
    assert_eq!(width_of(&deep), Size::Px(5.0));
}

#[test]
fn expressions_work_in_all_that_takes_a_length() {
    let page = styled(
        "p { font-size: 10px; margin: calc(1px + 1px) calc(3px * 2); padding-left: calc(5% + 1px);
             border: calc(1px + 1px) solid red; line-height: calc(1em + 2px);
             top: calc(1px + 1px); gap: calc(1px + 2px) }
         div { font-size: 20px }
         div p { font-size: calc(50% + 1px) }",
    );
    let style = page.style("a");
    assert_eq!(style.margin.top, Size::Px(2.0));
    assert_eq!(style.margin.right, Size::Px(6.0));
    assert!(matches!(style.padding.left, Size::Calc(_)));
    assert_eq!(style.border_width.top, 2.0);
    assert_eq!(style.offsets.top, Size::Px(2.0));
    assert_eq!(style.row_gap, 3.0);
    assert_eq!(style.font_size, 11.0);
    assert_eq!(style.line_height, LineHeight::Px(13.0));
}

#[test]
fn values_it_cannot_read_are_dropped() {
    assert_eq!(
        styled("p { color: red; color: var(--x) }").style("a").color,
        RED
    );
    assert_eq!(
        styled("p { width: 5px; width: calc(1px+1px) }")
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

/// The colors of all elements of this name in order, in a page where they are written alike.
fn colors_of(source: &str, name: &str) -> Vec<Color> {
    let page = Page::new(&format!("<!DOCTYPE html>{source}"));
    page.document
        .descendants(page.document.root())
        .filter(|&id| page.document.kind(id) == NodeKind::Element && page.document.name(id) == name)
        .map(|id| page.styles.get(id).color)
        .collect()
}

const BLACK: Color = Color::rgb(0, 0, 0);

#[test]
fn elements_written_alike_differ_by_what_is_around_them() {
    // By their ancestors, also those further up.
    assert_eq!(
        colors_of(
            "<style>section p{color:red}aside>div p{color:blue}</style>\
             <section><p>a</p></section><aside><div><p>b</p></div></aside><div><p>c</p></div>",
            "p"
        ),
        [RED, BLUE, BLACK]
    );
    // By an ancestor's attribute, which is all that differs in the tags above them.
    assert_eq!(
        colors_of(
            "<style>.x i{color:red}[data-y] i{color:blue}#z i{color:green}</style>\
             <p class=x><i>a</i></p><p data-y><i>b</i></p><p id=z><i>c</i></p><p><i>d</i></p>",
            "i"
        ),
        [RED, BLUE, GREEN, BLACK]
    );
}

#[test]
fn elements_written_alike_differ_by_their_neighbours() {
    assert_eq!(
        colors_of(
            "<style>p:first-child{color:red}p:last-child{color:blue}</style>\
             <div><p>a</p><p>b</p><p>c</p></div>",
            "p"
        ),
        [RED, BLACK, BLUE]
    );
    assert_eq!(
        colors_of(
            "<style>h1+p{color:red}h2~p{color:blue}</style>\
             <div><h1>x</h1><p>a</p><p>b</p></div><div><h2>y</h2><p>c</p><p>d</p></div>",
            "p"
        ),
        [RED, BLACK, BLUE, BLUE]
    );
    assert_eq!(
        colors_of(
            "<style>p:empty{color:red}p:nth-child(2){color:blue}</style>\
             <div><p></p><p>a</p></div>",
            "p"
        ),
        [RED, BLUE]
    );
    // Also when it is an ancestor that the neighbours decide for.
    assert_eq!(
        colors_of(
            "<style>p:first-child i{color:red}p:not(:first-child) i{color:blue}</style>\
             <div><p><i>a</i></p><p><i>b</i></p></div>",
            "i"
        ),
        [RED, BLUE]
    );
}

#[test]
fn rules_on_names_not_in_the_page_do_not_get_in_the_way() {
    assert_eq!(
        colors_of(
            "<style>.none,p{color:red}.none p{color:blue}#none{color:green}</style><p>a",
            "p"
        ),
        [RED]
    );
    // Quirks mode goes by names in any case.
    let page = parse("<style>.ABC{color:red}</style><p class=abc t=a>x");
    let sheets = document_stylesheets(&page, Viewport::default());
    let styles = compute_styles(&page, &sheets, Viewport::default());
    let paragraph = page
        .descendants(page.root())
        .find(|&id| page.name(id) == "p")
        .unwrap();
    assert_eq!(styles.get(paragraph).color, RED);
    // States of a live page do not hold, but their opposite does.
    assert_eq!(
        colors_of(
            "<style>p:hover{color:red}p:not(:hover){color:blue}</style><p>a",
            "p"
        ),
        [BLUE]
    );
    // Links are found also with no type in the selector.
    assert_eq!(
        colors_of(
            "<style>:link{color:red}</style><a href=x>a</a><a>b</a>",
            "a"
        ),
        [RED, BLACK]
    );
}

#[test]
fn variables_give_their_values() {
    let page = styled(
        ":root{--c:red;--w:7px;--s:1px solid blue} p{color:var(--c);margin-left:var(--w);\
         border:var(--s);background:var(--c)}",
    );
    let style = page.style("a");
    assert_eq!(style.color, RED);
    assert_eq!(style.margin.left, Size::Px(7.0));
    assert_eq!(style.background_color, RED);
    assert_eq!(style.border_style.left, BorderStyle::Solid);
    assert_eq!(style.border_color.left, BLUE);
}

#[test]
fn variables_inherit_and_can_change_on_the_way_down() {
    let page = Page::new(
        "<!DOCTYPE html><style>:root{--c:red} div{--c:blue} p,span,i,div{color:var(--c)}</style>\
         <p t=outer>x</p><div t=div><p t=inner>y<span t=deep>z</span></p></div>",
    );
    assert_eq!(page.style("outer").color, RED);
    assert_eq!(page.style("div").color, BLUE);
    assert_eq!(page.style("inner").color, BLUE);
    assert_eq!(page.style("deep").color, BLUE);
}

#[test]
fn a_variable_can_be_set_on_the_element_itself_and_in_its_style() {
    assert_eq!(styled("p{--c:red;color:var(--c)}").style("a").color, RED);
    let page = Page::new(
        "<!DOCTYPE html><style>p{--c:red;color:var(--c)}</style>\
         <p t=a style='--c:blue'>x</p><p t=b style='color:var(--c)'>y</p>",
    );
    assert_eq!(page.style("a").color, BLUE);
    assert_eq!(page.style("b").color, RED);
}

#[test]
fn fallbacks_and_missing_variables() {
    assert_eq!(styled("p{color:var(--none,red)}").style("a").color, RED);
    assert_eq!(
        styled("p{color:var(--none,var(--also-none,rgb(255,0,0)))}")
            .style("a")
            .color,
        RED
    );
    assert_eq!(
        styled("div{color:blue} p{color:var(--none)}")
            .style("a")
            .color,
        BLUE
    );
    // Names go by case.
    assert_eq!(
        styled("p{--C:red;color:blue;color:var(--c,green)}")
            .style("a")
            .color,
        GREEN
    );
    // The last of a property counts, also among those with variables.
    assert_eq!(
        styled("p{--c:red;color:var(--c);color:blue}")
            .style("a")
            .color,
        BLUE
    );
    assert_eq!(
        styled("p{--c:red;color:blue;color:var(--c)}")
            .style("a")
            .color,
        RED
    );
    assert_eq!(
        styled("p{--c:red;--c:blue;color:var(--c)}")
            .style("a")
            .color,
        BLUE
    );
    assert_eq!(
        styled("p{--c:red!important;--c:blue;color:var(--c)}")
            .style("a")
            .color,
        RED
    );
}

#[test]
fn rules_with_variables_keep_their_place_in_the_cascade() {
    assert_eq!(
        styled(":root{--c:red} p{color:blue} p{color:var(--c)}")
            .style("a")
            .color,
        RED
    );
    assert_eq!(
        styled(":root{--c:red} p{color:var(--c)} p{color:blue}")
            .style("a")
            .color,
        BLUE
    );
    assert_eq!(
        styled(":root{--c:red} #j{color:var(--c)} p{color:blue}")
            .style("a")
            .color,
        RED
    );
    assert_eq!(
        styled(":root{--c:red} p{color:var(--c)!important} #j{color:blue}")
            .style("a")
            .color,
        RED
    );
}

#[test]
fn variables_refer_to_each_other() {
    assert_eq!(
        styled(":root{--a:var(--b);--b:red} p{color:var(--a)}")
            .style("a")
            .color,
        RED
    );
    assert_eq!(
        styled(":root{--a:red} div{--b:var(--a)} p{--a:blue;color:var(--b)}")
            .style("a")
            .color,
        RED
    );
}

#[test]
fn what_only_looks_like_a_variable_is_left_alone() {
    let page = styled("p{--x:'var(--y)';font-family:'var(--z)'}");
    assert!(page.style("a").font_family.contains("var(--z)"));
    let customs = page.style("a").custom_properties.as_ref().unwrap();
    assert_eq!(customs.get("--x"), Some("'var(--y)'"));
    assert_eq!(styled("p{color:var(c,red)}").style("a").color, Color::BLACK);
}

#[test]
fn circles_and_floods_of_variables_end() {
    let start = std::time::Instant::now();
    assert_eq!(
        styled(
            "div{color:blue} :root{--a:var(--b);--b:var(--a);--self:var(--self)}\
                p{color:var(--a)}"
        )
        .style("a")
        .color,
        BLUE
    );
    // Each doubles the one before.
    let mut flood = String::from(":root{--v0:red;");
    for level in 1..60 {
        let last = level - 1;
        flood += &format!("--v{level}:var(--v{last}) var(--v{last});");
    }
    flood += "} p{color:var(--v59);margin-left:7px}";
    assert_eq!(styled(&flood).style("a").margin.left, Size::Px(7.0));
    // Fallbacks in fallbacks.
    let deep = format!(
        "p{{color:{}red{}}}",
        "var(--n,".repeat(5000),
        ")".repeat(5000)
    );
    let _ = styled(&deep).style("a").color;
    // A new set of values for each of many elements.
    let many: String = (0..3000)
        .map(|i| format!("<p style='--x{i}:{}'>", "y".repeat(1000)))
        .collect();
    let _ = Page::new(&format!("<!DOCTYPE html>{many}"));
    assert!(start.elapsed().as_secs() < 20);
}
