//! The document tree: nodes in an array, with text and attributes kept in the source.

use std::borrow::Cow;

use super::attributes::{Attribute, Attributes, attributes};
use super::doctype::Doctype;
use super::entities::{Context, decode};
use super::tag::Tag;
use super::tokenizer::tag_name;

const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeId(u32);

impl NodeId {
    /// The position in the document's node array, for tables indexed by node.
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Document,
    Doctype,
    Element,
    Text,
    Comment,
    /// The inert contents of a `<template>`, held as its only child.
    Fragment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}

/// Twenty-eight bytes per node keep the tree close to the size of the source. The first
/// child's `previous` holds the last child, which makes appending constant-time.
#[derive(Clone, Copy)]
struct Node {
    parent: u32,
    first_child: u32,
    next: u32,
    previous: u32,
    start: u32,
    length: u32,
    tag: Tag,
    kind: NodeKind,
    namespace: Namespace,
    /// On the tree builder's stack of open elements.
    open: bool,
}

const _: () = assert!(size_of::<Node>() == 28);

/// A document, complete or as far as it has arrived. Elements still open are simply
/// the last on their paths, so the tree is valid to show at any moment.
pub struct Document {
    pub(super) source: String,
    nodes: Vec<Node>,
    /// Tags whose attributes were added to an existing `<html>` or `<body>`, as
    /// (element, start, end) in the source.
    merged_attributes: Vec<(u32, u32, u32)>,
    quirks: bool,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub(super) fn new() -> Self {
        let mut document = Self {
            source: String::new(),
            nodes: Vec::new(),
            merged_attributes: Vec::new(),
            quirks: false,
        };
        document.create(NodeKind::Document, Tag::Other, 0..0);
        document
    }

    pub(super) fn create(
        &mut self,
        kind: NodeKind,
        tag: Tag,
        span: std::ops::Range<usize>,
    ) -> NodeId {
        // The parser keeps sources under 4 GiB and node counts under its limit.
        let id = NodeId(u32::try_from(self.nodes.len()).unwrap_or(NONE - 1));
        self.nodes.push(Node {
            parent: NONE,
            first_child: NONE,
            next: NONE,
            previous: NONE,
            start: u32::try_from(span.start).unwrap_or(u32::MAX),
            length: u32::try_from(span.len()).unwrap_or(u32::MAX),
            tag,
            kind,
            namespace: Namespace::Html,
            open: false,
        });
        id
    }

    pub(super) fn clone_element(&mut self, id: NodeId) -> NodeId {
        let node = self.node(id);
        let start = node.start as usize;
        let (kind, tag, length) = (node.kind, node.tag, node.length as usize);
        let namespace = node.namespace;
        let clone = self.create(kind, tag, start..start + length);
        self.set_namespace(clone, namespace);
        clone
    }

    pub(super) fn append(&mut self, parent: NodeId, child: NodeId) {
        let first = self.node(parent).first_child;
        self.nodes[child.0 as usize].parent = parent.0;
        if first == NONE {
            self.nodes[parent.0 as usize].first_child = child.0;
            self.nodes[child.0 as usize].previous = child.0;
        } else {
            let last = self.nodes[first as usize].previous;
            self.nodes[last as usize].next = child.0;
            self.nodes[child.0 as usize].previous = last;
            self.nodes[first as usize].previous = child.0;
        }
    }

    pub(super) fn set_namespace(&mut self, id: NodeId, namespace: Namespace) {
        self.nodes[id.0 as usize].namespace = namespace;
    }

    pub(super) fn set_quirks(&mut self, quirks: bool) {
        self.quirks = quirks;
    }

    /// Adds the attributes of a later tag to an element, without replacing any.
    pub(super) fn merge_attributes(&mut self, id: NodeId, span: &std::ops::Range<usize>) {
        let start = u32::try_from(span.start).unwrap_or(u32::MAX);
        let end = u32::try_from(span.end).unwrap_or(u32::MAX);
        self.merged_attributes.push((id.0, start, end));
    }

    /// Inserts `child` in front of `before`, a child of `parent`.
    pub(super) fn insert_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) {
        let previous = self.node(before).previous;
        self.nodes[child.0 as usize].parent = parent.0;
        self.nodes[child.0 as usize].next = before.0;
        self.nodes[child.0 as usize].previous = previous;
        self.nodes[before.0 as usize].previous = child.0;
        if self.nodes[parent.0 as usize].first_child == before.0 {
            self.nodes[parent.0 as usize].first_child = child.0;
        } else {
            self.nodes[previous as usize].next = child.0;
        }
    }

    /// The child in front of `id`.
    #[must_use]
    pub fn previous_sibling(&self, id: NodeId) -> Option<NodeId> {
        let node = self.node(id);
        let parent = Self::link(node.parent)?;
        (self.node(parent).first_child != id.0)
            .then(|| Self::link(node.previous))
            .flatten()
    }

    pub(super) fn detach(&mut self, id: NodeId) {
        let Node {
            parent,
            next,
            previous,
            ..
        } = *self.node(id);
        if parent == NONE {
            return;
        }
        if self.nodes[parent as usize].first_child == id.0 {
            self.nodes[parent as usize].first_child = next;
            if next != NONE {
                self.nodes[next as usize].previous = previous;
            }
        } else {
            self.nodes[previous as usize].next = next;
            let after = if next == NONE {
                self.nodes[parent as usize].first_child
            } else {
                next
            };
            self.nodes[after as usize].previous = previous;
        }
        let node = &mut self.nodes[id.0 as usize];
        node.parent = NONE;
        node.next = NONE;
        node.previous = NONE;
    }

    pub(super) fn set_open(&mut self, id: NodeId, open: bool) {
        self.nodes[id.0 as usize].open = open;
    }

    pub(super) fn is_open(&self, id: NodeId) -> bool {
        self.node(id).open
    }

    /// Extends a text node when the new piece directly follows it in the source.
    pub(super) fn extend_text(&mut self, id: NodeId, span: &std::ops::Range<usize>) -> bool {
        let node = &mut self.nodes[id.0 as usize];
        if node.kind != NodeKind::Text || (node.start + node.length) as usize != span.start {
            return false;
        }
        node.length += u32::try_from(span.len()).unwrap_or(0);
        true
    }

    pub(super) fn shrink_to_fit(&mut self) {
        self.source.shrink_to_fit();
        self.nodes.shrink_to_fit();
        self.merged_attributes.shrink_to_fit();
    }

    fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    fn link(value: u32) -> Option<NodeId> {
        (value != NONE).then_some(NodeId(value))
    }

    #[must_use]
    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    #[must_use]
    pub fn kind(&self, id: NodeId) -> NodeKind {
        self.node(id).kind
    }

    #[must_use]
    pub fn tag(&self, id: NodeId) -> Tag {
        self.node(id).tag
    }

    #[must_use]
    pub fn namespace(&self, id: NodeId) -> Namespace {
        self.node(id).namespace
    }

    /// Whether the doctype, or the lack of one, selects the rendering of old browsers.
    #[must_use]
    pub fn is_quirks(&self) -> bool {
        self.quirks
    }

    #[must_use]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        Self::link(self.node(id).parent)
    }

    #[must_use]
    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        Self::link(self.node(id).first_child)
    }

    #[must_use]
    pub fn last_child(&self, id: NodeId) -> Option<NodeId> {
        let first = self.first_child(id)?;
        Self::link(self.node(first).previous)
    }

    #[must_use]
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        Self::link(self.node(id).next)
    }

    /// The source text of the node: the start tag of an element, which is empty when
    /// the element was implied, or the raw text of a text node or comment.
    #[must_use]
    pub fn text(&self, id: NodeId) -> &str {
        let node = self.node(id);
        let start = node.start as usize;
        &self.source[start..start + node.length as usize]
    }

    /// The content of a text node as it should be shown: line ends normalized and
    /// character references decoded, except in elements whose content is not markup.
    #[must_use]
    pub fn decoded_text(&self, id: NodeId) -> Cow<'_, str> {
        let raw = self.text(id);
        let parent = self.parent(id);
        let tag = parent.map_or(Tag::Other, |parent| self.tag(parent));
        let foreign = parent.is_some_and(|parent| self.namespace(parent) != Namespace::Html)
            && !tag.is_html_integration_point()
            && !tag.is_math_text_integration_point();
        let cleaned = if raw.contains(['\r', '\0']) {
            let lines = raw.replace("\r\n", "\n").replace('\r', "\n");
            let replacement = if tag.is_raw_text() || foreign {
                "\u{fffd}"
            } else {
                ""
            };
            Cow::Owned(lines.replace('\0', replacement))
        } else {
            Cow::Borrowed(raw)
        };
        if tag.is_raw_text() && !matches!(tag, Tag::Title | Tag::Textarea) {
            return cleaned;
        }
        match cleaned {
            Cow::Borrowed(text) => decode(text, Context::Text),
            Cow::Owned(text) => Cow::Owned(decode(&text, Context::Text).into_owned()),
        }
    }

    /// The text of a comment, without its delimiters.
    #[must_use]
    pub fn comment_text(&self, id: NodeId) -> Cow<'_, str> {
        let raw = self.text(id);
        let text = if let Some(body) = raw.strip_prefix("<!--") {
            if matches!(body, ">" | "->") {
                ""
            } else if let Some(text) = body.strip_suffix("-->").or(body.strip_suffix("--!>")) {
                text
            } else {
                // The input ended inside the comment.
                body.strip_suffix("--")
                    .or(body.strip_suffix('-'))
                    .unwrap_or(body)
            }
        } else {
            let body = raw.strip_suffix('>').unwrap_or(raw);
            body.strip_prefix("</")
                .or(body.strip_prefix("<!"))
                .or(body.strip_prefix('<'))
                .unwrap_or(body)
        };
        if text.contains(['\r', '\0']) {
            let lines = text.replace("\r\n", "\n").replace('\r', "\n");
            Cow::Owned(lines.replace('\0', "\u{fffd}"))
        } else {
            Cow::Borrowed(text)
        }
    }

    /// Reads a doctype node.
    #[must_use]
    pub fn doctype(&self, id: NodeId) -> Doctype {
        Doctype::parse(self.text(id))
    }

    /// The element name: lowercase for known tags, as written otherwise.
    #[must_use]
    pub fn name(&self, id: NodeId) -> &str {
        match self.tag(id) {
            tag if tag != Tag::Other && self.namespace(id) == Namespace::Html => tag.as_str(),
            _ => tag_name(self.text(id)),
        }
    }

    /// The name as the DOM reports it: lowercase, with the mixed case of known SVG names.
    #[must_use]
    pub fn local_name(&self, id: NodeId) -> Cow<'_, str> {
        let name = self.name(id);
        if self.namespace(id) == Namespace::Svg {
            let lower = name.to_ascii_lowercase();
            if let Some((_, adjusted)) = SVG_NAMES.iter().find(|(from, _)| *from == lower) {
                return Cow::Borrowed(adjusted);
            }
            return Cow::Owned(lower);
        }
        if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
            Cow::Owned(name.to_ascii_lowercase())
        } else {
            Cow::Borrowed(name)
        }
    }

    /// Reads the attributes of an element from its start tag.
    #[must_use]
    pub fn attributes(&self, id: NodeId) -> ElementAttributes<'_> {
        ElementAttributes {
            document: self,
            id: id.0,
            current: attributes(self.text(id)),
            next_merged: 0,
        }
    }

    #[must_use]
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            document: self,
            next: self.first_child(id),
        }
    }

    /// Walks the tree below and including `id` in document order, without recursion.
    #[must_use]
    pub fn descendants(&self, id: NodeId) -> Descendants<'_> {
        Descendants {
            document: self,
            root: id,
            next: Some(id),
        }
    }

    /// The raw content of the first `<title>` in document order.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        let title = self
            .descendants(self.root())
            .find(|&id| self.tag(id) == Tag::Title)?;
        Some(self.first_child(title).map_or("", |text| self.text(text)))
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Bytes allocated for the nodes.
    #[must_use]
    pub fn nodes_capacity_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<Node>()
    }

    /// Bytes allocated for the source text.
    #[must_use]
    pub fn source_capacity_bytes(&self) -> usize {
        self.source.capacity()
    }
}

/// The attributes of an element: those of its tag, then any added by later tags.
pub struct ElementAttributes<'a> {
    document: &'a Document,
    id: u32,
    current: Attributes<'a>,
    next_merged: usize,
}

impl<'a> Iterator for ElementAttributes<'a> {
    type Item = Attribute<'a>;

    fn next(&mut self) -> Option<Attribute<'a>> {
        loop {
            if let Some(attribute) = self.current.next() {
                return Some(attribute);
            }
            let &(id, start, end) = self.document.merged_attributes.get(self.next_merged)?;
            self.next_merged += 1;
            if id == self.id {
                let text = &self.document.source[start as usize..end as usize];
                self.current = attributes(text);
            }
        }
    }
}

const SVG_NAMES: [(&str, &str); 37] = [
    ("altglyph", "altGlyph"),
    ("altglyphdef", "altGlyphDef"),
    ("altglyphitem", "altGlyphItem"),
    ("animatecolor", "animateColor"),
    ("animatemotion", "animateMotion"),
    ("animatetransform", "animateTransform"),
    ("clippath", "clipPath"),
    ("feblend", "feBlend"),
    ("fecolormatrix", "feColorMatrix"),
    ("fecomponenttransfer", "feComponentTransfer"),
    ("fecomposite", "feComposite"),
    ("feconvolvematrix", "feConvolveMatrix"),
    ("fediffuselighting", "feDiffuseLighting"),
    ("fedisplacementmap", "feDisplacementMap"),
    ("fedistantlight", "feDistantLight"),
    ("fedropshadow", "feDropShadow"),
    ("feflood", "feFlood"),
    ("fefunca", "feFuncA"),
    ("fefuncb", "feFuncB"),
    ("fefuncg", "feFuncG"),
    ("fefuncr", "feFuncR"),
    ("fegaussianblur", "feGaussianBlur"),
    ("feimage", "feImage"),
    ("femerge", "feMerge"),
    ("femergenode", "feMergeNode"),
    ("femorphology", "feMorphology"),
    ("feoffset", "feOffset"),
    ("fepointlight", "fePointLight"),
    ("fespecularlighting", "feSpecularLighting"),
    ("fespotlight", "feSpotLight"),
    ("fetile", "feTile"),
    ("feturbulence", "feTurbulence"),
    ("foreignobject", "foreignObject"),
    ("glyphref", "glyphRef"),
    ("lineargradient", "linearGradient"),
    ("radialgradient", "radialGradient"),
    ("textpath", "textPath"),
];

pub struct Children<'a> {
    document: &'a Document,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let id = self.next?;
        self.next = self.document.next_sibling(id);
        Some(id)
    }
}

pub struct Descendants<'a> {
    document: &'a Document,
    root: NodeId,
    next: Option<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let id = self.next?;
        let document = self.document;
        self.next = document.first_child(id).or_else(|| {
            let mut node = id;
            loop {
                if node == self.root {
                    return None;
                }
                if let Some(sibling) = document.next_sibling(node) {
                    return Some(sibling);
                }
                node = document.parent(node)?;
            }
        });
        Some(id)
    }
}
