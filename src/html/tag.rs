//! Element names the tree builder treats specially, as small integers.

macro_rules! tags {
    ($($variant:ident $text:literal)*) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(u8)]
        pub enum Tag {
            /// Any other name; it can be read from the source.
            Other,
            $($variant,)*
        }

        impl Tag {
            /// Finds the tag for a name in any letter case.
            #[must_use]
            pub fn from_name(name: &str) -> Self {
                let mut buffer = [0_u8; 12];
                let Some(lower) = buffer.get_mut(..name.len()) else {
                    return Self::Other;
                };
                lower.copy_from_slice(name.as_bytes());
                lower.make_ascii_lowercase();
                match std::str::from_utf8(lower).unwrap_or("") {
                    $($text => Self::$variant,)*
                    _ => Self::Other,
                }
            }

            /// The lowercase name; empty for [`Tag::Other`].
            #[must_use]
            pub fn as_str(self) -> &'static str {
                match self {
                    Self::Other => "",
                    $(Self::$variant => $text,)*
                }
            }
        }
    };
}

tags! {
    A "a" Address "address" Applet "applet" Area "area" Article "article" Aside "aside"
    B "b" Base "base" Basefont "basefont" Bgsound "bgsound" Big "big" Blockquote "blockquote"
    Body "body" Br "br" Button "button" Caption "caption" Center "center" Code "code"
    Col "col" Colgroup "colgroup" Dd "dd" Details "details" Dialog "dialog" Dir "dir"
    Div "div" Dl "dl" Dt "dt" Em "em" Embed "embed" Fieldset "fieldset"
    Figcaption "figcaption" Figure "figure" Font "font" Footer "footer" Form "form"
    Frame "frame" Frameset "frameset" H1 "h1" H2 "h2" H3 "h3" H4 "h4" H5 "h5" H6 "h6"
    Head "head" Header "header" Hgroup "hgroup" Hr "hr" Html "html" I "i" Iframe "iframe"
    Img "img" Input "input" Keygen "keygen" Li "li" Link "link" Listing "listing"
    Main "main" Marquee "marquee" Math "math" Menu "menu" Meta "meta" Nav "nav" Nobr "nobr"
    Noembed "noembed" Noframes "noframes" Noscript "noscript" Object "object" Ol "ol"
    Optgroup "optgroup" Option "option" P "p" Param "param" Plaintext "plaintext" Pre "pre"
    Rb "rb" Rp "rp" Rt "rt" Rtc "rtc" S "s" Script "script" Search "search" Section "section"
    Select "select" Small "small" Source "source" Strike "strike" Strong "strong"
    Style "style" Summary "summary" Svg "svg" Table "table" Tbody "tbody" Td "td"
    Template "template" Textarea "textarea" Tfoot "tfoot" Th "th" Thead "thead"
    Title "title" Tr "tr" Track "track" Tt "tt" U "u" Ul "ul" Wbr "wbr" Xmp "xmp"
}

impl Tag {
    /// Elements that never have content.
    pub(super) fn is_void(self) -> bool {
        use Tag::{
            Area, Base, Basefont, Bgsound, Br, Col, Embed, Frame, Hr, Img, Input, Keygen, Link,
            Meta, Param, Source, Track, Wbr,
        };
        matches!(
            self,
            Area | Base
                | Basefont
                | Bgsound
                | Br
                | Col
                | Embed
                | Frame
                | Hr
                | Img
                | Input
                | Keygen
                | Link
                | Meta
                | Param
                | Source
                | Track
                | Wbr
        )
    }

    pub(super) fn is_heading(self) -> bool {
        matches!(
            self,
            Tag::H1 | Tag::H2 | Tag::H3 | Tag::H4 | Tag::H5 | Tag::H6
        )
    }

    /// Elements that a misnested formatting element is carried across.
    pub(super) fn is_formatting(self) -> bool {
        use Tag::{A, B, Big, Code, Em, Font, I, Nobr, S, Small, Strike, Strong, Tt, U};
        matches!(
            self,
            A | B | Big | Code | Em | Font | I | Nobr | S | Small | Strike | Strong | Tt | U
        )
    }

    /// Start tags that close an open `<p>` and open a block.
    pub(super) fn closes_paragraph(self) -> bool {
        use Tag::{
            Address, Article, Aside, Blockquote, Center, Details, Dialog, Dir, Div, Dl, Fieldset,
            Figcaption, Figure, Footer, Header, Hgroup, Main, Menu, Nav, Ol, P, Search, Section,
            Summary, Ul,
        };
        matches!(
            self,
            Address
                | Article
                | Aside
                | Blockquote
                | Center
                | Details
                | Dialog
                | Dir
                | Div
                | Dl
                | Fieldset
                | Figcaption
                | Figure
                | Footer
                | Header
                | Hgroup
                | Main
                | Menu
                | Nav
                | Ol
                | P
                | Search
                | Section
                | Summary
                | Ul
        )
    }

    /// Elements an end tag closes only when they are in scope, with implied ends.
    pub(super) fn closes_as_block(self) -> bool {
        (self.closes_paragraph() && self != Tag::P)
            || matches!(self, Tag::Pre | Tag::Listing | Tag::Button)
    }

    /// The HTML "special" category, which stops searches for an element.
    pub(super) fn is_special(self) -> bool {
        use Tag::{
            Applet, Area, Base, Basefont, Bgsound, Body, Br, Button, Caption, Col, Colgroup, Dd,
            Dt, Embed, Form, Frame, Frameset, Head, Hr, Html, Iframe, Img, Input, Keygen, Li, Link,
            Listing, Marquee, Meta, Noembed, Noframes, Noscript, Object, Param, Plaintext, Pre,
            Script, Select, Source, Style, Table, Tbody, Td, Template, Textarea, Tfoot, Th, Thead,
            Title, Tr, Track, Wbr, Xmp,
        };
        (self.closes_paragraph() && self != Tag::P)
            || self.is_heading()
            || matches!(
                self,
                Tag::P
                    | Applet
                    | Area
                    | Base
                    | Basefont
                    | Bgsound
                    | Body
                    | Br
                    | Button
                    | Caption
                    | Col
                    | Colgroup
                    | Dd
                    | Dt
                    | Embed
                    | Form
                    | Frame
                    | Frameset
                    | Head
                    | Hr
                    | Html
                    | Iframe
                    | Img
                    | Input
                    | Keygen
                    | Li
                    | Link
                    | Listing
                    | Marquee
                    | Meta
                    | Noembed
                    | Noframes
                    | Noscript
                    | Object
                    | Param
                    | Plaintext
                    | Pre
                    | Script
                    | Select
                    | Source
                    | Style
                    | Table
                    | Tbody
                    | Td
                    | Template
                    | Textarea
                    | Tfoot
                    | Th
                    | Thead
                    | Title
                    | Tr
                    | Track
                    | Wbr
                    | Xmp
            )
    }

    /// Elements with an end tag that a following tag may leave out.
    pub(super) fn has_implied_end(self) -> bool {
        use Tag::{Dd, Dt, Li, Optgroup, Option, P, Rb, Rp, Rt, Rtc};
        matches!(
            self,
            Dd | Dt | Li | Optgroup | Option | P | Rb | Rp | Rt | Rtc
        )
    }

    /// Elements whose content the tokenizer returns as raw text.
    pub(super) fn is_raw_text(self) -> bool {
        use Tag::{Iframe, Noembed, Noframes, Plaintext, Script, Style, Textarea, Title, Xmp};
        matches!(
            self,
            Iframe | Noembed | Noframes | Plaintext | Script | Style | Textarea | Title | Xmp
        )
    }

    /// Elements that belong in `<head>`.
    pub(super) fn is_head_content(self) -> bool {
        use Tag::{
            Base, Basefont, Bgsound, Link, Meta, Noframes, Noscript, Script, Style, Template, Title,
        };
        matches!(
            self,
            Base | Basefont
                | Bgsound
                | Link
                | Meta
                | Noframes
                | Noscript
                | Script
                | Style
                | Template
                | Title
        )
    }
}
