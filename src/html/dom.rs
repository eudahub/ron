//! The document tree: nodes in an array, with text and attributes kept in the source.

use super::attributes::{Attributes, attributes};
use super::tag::Tag;
use super::tokenizer::tag_name;

const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeId(u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Document,
    Doctype,
    Element,
    Text,
    Comment,
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
    /// On the tree builder's stack of open elements.
    open: bool,
}

const _: () = assert!(size_of::<Node>() == 28);

/// A document, complete or as far as it has arrived. Elements still open are simply
/// the last on their paths, so the tree is valid to show at any moment.
pub struct Document {
    pub(super) source: String,
    nodes: Vec<Node>,
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
            open: false,
        });
        id
    }

    pub(super) fn clone_element(&mut self, id: NodeId) -> NodeId {
        let node = self.node(id);
        let start = node.start as usize;
        let (kind, tag, length) = (node.kind, node.tag, node.length as usize);
        self.create(kind, tag, start..start + length)
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

    /// The element name: lowercase for known tags, as written otherwise.
    #[must_use]
    pub fn name(&self, id: NodeId) -> &str {
        match self.tag(id) {
            Tag::Other => tag_name(self.text(id)),
            tag => tag.as_str(),
        }
    }

    /// Reads the attributes of an element from its start tag.
    #[must_use]
    pub fn attributes(&self, id: NodeId) -> Attributes<'_> {
        attributes(self.text(id))
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
