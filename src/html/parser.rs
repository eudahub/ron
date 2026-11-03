//! Builds the document tree from tokens, one at a time, following the HTML tree
//! construction rules. Not implemented yet: moving misplaced table content in front of
//! the table, `<template>` contents, and the separate parsing rules of SVG and `MathML`.

use std::ops::Range;

use super::dom::{Document, NodeId, NodeKind};
use super::tag::Tag;
use super::tokenizer::{Scanner, Token, TokenKind, is_html_space, tag_name};

/// Limit of open elements; further ones become siblings. Searches walk the open elements,
/// so this bounds them. The tree can get deeper, so code walking it must not recurse.
const MAX_DEPTH: usize = 512;
/// The oldest entries are dropped beyond this; it bounds the elements one text can reopen.
const MAX_ACTIVE_FORMATTING: usize = 64;
/// The tree stops growing here, so that small hostile input cannot exhaust memory.
const MAX_NODES: usize = 1 << 24;
/// One token can create this many nodes at most.
const NODES_PER_TOKEN: usize = 1024;
/// Spans are stored in 32 bits.
const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
    AfterBody,
}

/// The newline right after `<pre>`, `<listing>` or `<textarea>` is not content.
#[derive(Clone, Copy, Eq, PartialEq)]
enum NewlineSkip {
    No,
    Next,
    /// A piece ended after `\r`; a `\n` at the start of the next one is part of it.
    AfterCarriageReturn,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Active {
    Marker,
    Element(NodeId),
}

/// A start or end tag being processed.
#[derive(Clone, Copy)]
struct TagToken {
    tag: Tag,
    start: usize,
    end: usize,
    name_start: usize,
    name_end: usize,
    self_closing: bool,
}

impl TagToken {
    fn span(self) -> Range<usize> {
        self.start..self.end
    }
}

/// Parses a document that arrives in pieces. After each [`Parser::feed`] the tree holds
/// everything that can already be known and may be shown. Tokens are not kept.
pub struct Parser {
    document: Document,
    scanner: Scanner,
    finished: bool,
    truncated: bool,
    mode: Mode,
    open: Vec<NodeId>,
    active: Vec<Active>,
    head: Option<NodeId>,
    form: Option<NodeId>,
    skip_newline: NewlineSkip,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    #[must_use]
    pub fn new() -> Self {
        Self {
            document: Document::new(),
            scanner: Scanner::default(),
            finished: false,
            truncated: false,
            mode: Mode::Initial,
            open: Vec::new(),
            active: Vec::new(),
            head: None,
            form: None,
            skip_newline: NewlineSkip::No,
        }
    }

    /// Reserves the source buffer when the document size is known.
    #[must_use]
    pub fn with_capacity(bytes: usize) -> Self {
        let mut parser = Self::new();
        parser.document.source.reserve(bytes);
        parser
    }

    /// Adds the next piece of the document and grows the tree as far as it allows.
    ///
    /// # Panics
    ///
    /// Panics when called after [`Self::finish`].
    pub fn feed(&mut self, piece: &str) {
        assert!(!self.finished, "feed called after finish");
        if self.document.source.len() + piece.len() > MAX_SOURCE_BYTES {
            self.truncated = true;
        }
        if !self.truncated {
            self.document.source.push_str(piece);
            self.process_tokens();
        }
    }

    /// Ends the document: completes waiting tokens and adds any elements still missing.
    pub fn finish(&mut self) {
        self.finished = true;
        self.process_tokens();
        while !matches!(self.mode, Mode::InBody | Mode::AfterBody) {
            self.advance_mode();
        }
        self.document.shrink_to_fit();
        self.open = Vec::new();
        self.active = Vec::new();
    }

    /// The tree as far as it has been built.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }

    #[must_use]
    pub fn into_document(self) -> Document {
        self.document
    }

    /// Whether a limit stopped parsing before the end of the document.
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    fn process_tokens(&mut self) {
        while !self.truncated {
            if self.document.node_count() + NODES_PER_TOKEN > MAX_NODES {
                self.truncated = true;
                return;
            }
            let Some(token) = self.scanner.next(&self.document.source, self.finished) else {
                return;
            };
            self.process(token);
        }
    }

    fn process(&mut self, token: Token) {
        let Token { kind, mut span } = token;
        let skip = std::mem::replace(&mut self.skip_newline, NewlineSkip::No);
        if skip != NewlineSkip::No && matches!(kind, TokenKind::Text | TokenKind::RawText) {
            let source = &self.document.source;
            if source[span.clone()].starts_with('\n') {
                span.start += 1;
            } else if skip == NewlineSkip::Next && source[span.clone()].starts_with('\r') {
                span.start += 1;
                if span.is_empty() {
                    self.skip_newline = NewlineSkip::AfterCarriageReturn;
                } else if source[span.clone()].starts_with('\n') {
                    span.start += 1;
                }
            }
            if span.is_empty() {
                return;
            }
        }
        match kind {
            TokenKind::RawText => self.insert_text(span),
            TokenKind::Text | TokenKind::StrayLt => self.process_text(span),
            TokenKind::Comment => self.insert_comment(span),
            TokenKind::Declaration => self.process_declaration(span),
            TokenKind::StartTag => {
                let tag = self.tag_token(span);
                self.process_start_tag(tag);
            }
            TokenKind::EndTag => {
                let tag = self.tag_token(span);
                self.process_end_tag(tag);
            }
        }
    }

    fn tag_token(&self, span: Range<usize>) -> TagToken {
        let text = &self.document.source[span.clone()];
        let name = tag_name(text);
        let prefix = if text.starts_with("</") { 2 } else { 1 };
        TagToken {
            tag: Tag::from_name(name),
            start: span.start,
            end: span.end,
            name_start: span.start + prefix,
            name_end: span.start + prefix + name.len(),
            self_closing: text.ends_with("/>"),
        }
    }

    /// Moves to the next mode when a token does not belong to the current one, creating
    /// the elements the document left out.
    fn advance_mode(&mut self) {
        match self.mode {
            Mode::Initial => self.mode = Mode::BeforeHtml,
            Mode::BeforeHtml => {
                self.insert_implied(Tag::Html);
                self.mode = Mode::BeforeHead;
            }
            Mode::BeforeHead => {
                self.head = Some(self.insert_implied(Tag::Head));
                self.mode = Mode::InHead;
            }
            Mode::InHead => {
                self.pop_until(|tag| tag == Tag::Head);
                self.mode = Mode::AfterHead;
            }
            Mode::AfterHead => {
                self.insert_implied(Tag::Body);
                self.mode = Mode::InBody;
            }
            Mode::InBody | Mode::AfterBody => self.mode = Mode::InBody,
        }
    }

    fn process_text(&mut self, mut span: Range<usize>) {
        loop {
            match self.mode {
                Mode::Initial | Mode::BeforeHtml | Mode::BeforeHead => {
                    span.start += self.leading_space(&span);
                    if span.is_empty() {
                        return;
                    }
                }
                Mode::InHead | Mode::AfterHead => {
                    let space = self.leading_space(&span);
                    if space > 0 {
                        self.insert_text(span.start..span.start + space);
                        span.start += space;
                    }
                    if span.is_empty() {
                        return;
                    }
                }
                Mode::AfterBody if self.leading_space(&span) == span.len() => {
                    self.insert_text(span);
                    return;
                }
                Mode::InBody => {
                    self.reconstruct_active_formatting();
                    self.insert_text(span);
                    return;
                }
                Mode::AfterBody => {}
            }
            self.advance_mode();
        }
    }

    fn leading_space(&self, span: &Range<usize>) -> usize {
        let text = &self.document.source[span.clone()];
        text.len() - text.trim_start_matches(is_html_space).len()
    }

    fn process_declaration(&mut self, span: Range<usize>) {
        let text = &self.document.source[span.clone()];
        if text == "</>" {
            return;
        }
        let is_doctype = text
            .get(..9)
            .is_some_and(|start| start.eq_ignore_ascii_case("<!doctype"));
        if !is_doctype {
            self.insert_comment(span);
        } else if self.mode == Mode::Initial {
            let doctype = self.document.create(NodeKind::Doctype, Tag::Other, span);
            self.document.append(self.document.root(), doctype);
            self.mode = Mode::BeforeHtml;
        }
    }

    fn process_start_tag(&mut self, token: TagToken) {
        let tag = token.tag;
        loop {
            match self.mode {
                Mode::Initial => {}
                Mode::BeforeHtml => {
                    if tag == Tag::Html {
                        self.insert_element(token);
                        self.mode = Mode::BeforeHead;
                        return;
                    }
                }
                Mode::BeforeHead => match tag {
                    Tag::Html => return,
                    Tag::Head => {
                        self.head = Some(self.insert_element(token));
                        self.mode = Mode::InHead;
                        return;
                    }
                    _ => {}
                },
                Mode::InHead => {
                    if matches!(tag, Tag::Html | Tag::Head) {
                        return;
                    }
                    if tag.is_head_content() {
                        self.insert_head_content(token);
                        return;
                    }
                }
                Mode::AfterHead => {
                    if matches!(tag, Tag::Html | Tag::Head) {
                        return;
                    }
                    if tag == Tag::Body {
                        self.insert_element(token);
                        self.mode = Mode::InBody;
                        return;
                    }
                    if tag.is_head_content() {
                        // Late head content still goes into the head.
                        if let Some(head) = self.head {
                            self.push_open(head);
                            self.insert_head_content(token);
                            if let Some(index) = self.open.iter().position(|&id| id == head) {
                                self.document.set_open(head, false);
                                self.open.remove(index);
                            }
                        }
                        return;
                    }
                }
                Mode::InBody => {
                    self.start_tag_in_body(token);
                    return;
                }
                Mode::AfterBody => {}
            }
            self.advance_mode();
        }
    }

    fn process_end_tag(&mut self, token: TagToken) {
        let tag = token.tag;
        if self.current_tag().is_raw_text() && self.same_name(self.current(), token) {
            self.pop();
            return;
        }
        loop {
            match self.mode {
                Mode::Initial | Mode::BeforeHtml | Mode::BeforeHead => {
                    if !matches!(tag, Tag::Head | Tag::Body | Tag::Html | Tag::Br) {
                        return;
                    }
                }
                Mode::InHead => {
                    if tag == Tag::Head {
                        self.advance_mode();
                        return;
                    }
                    if !matches!(tag, Tag::Body | Tag::Html | Tag::Br) {
                        if self.same_name(self.current(), token) {
                            self.pop();
                        }
                        return;
                    }
                }
                Mode::AfterHead => {
                    if !matches!(tag, Tag::Body | Tag::Html | Tag::Br) {
                        return;
                    }
                }
                Mode::InBody => {
                    self.end_tag_in_body(token);
                    return;
                }
                Mode::AfterBody => {
                    if tag == Tag::Html {
                        return;
                    }
                }
            }
            self.advance_mode();
        }
    }

    fn insert_head_content(&mut self, token: TagToken) {
        if token.tag.is_void() {
            self.insert_void(token);
        } else {
            self.insert_element(token);
        }
    }

    fn start_tag_in_body(&mut self, token: TagToken) {
        let tag = token.tag;
        match tag {
            Tag::Html | Tag::Head | Tag::Body => {}
            _ if tag.is_head_content() => self.insert_head_content(token),
            Tag::H1 | Tag::H2 | Tag::H3 | Tag::H4 | Tag::H5 | Tag::H6 => {
                self.close_paragraph();
                if self.current_tag().is_heading() {
                    self.pop();
                }
                self.insert_element(token);
            }
            Tag::Pre | Tag::Listing => {
                self.close_paragraph();
                self.insert_element(token);
                self.skip_newline = NewlineSkip::Next;
            }
            Tag::Form => {
                if self.form.is_none() {
                    self.close_paragraph();
                    self.form = Some(self.insert_element(token));
                }
            }
            Tag::Li => {
                self.close_list_item(|tag| tag == Tag::Li);
                self.insert_element(token);
            }
            Tag::Dd | Tag::Dt => {
                self.close_list_item(|tag| matches!(tag, Tag::Dd | Tag::Dt));
                self.insert_element(token);
            }
            Tag::Plaintext => {
                self.close_paragraph();
                self.insert_element(token);
            }
            Tag::Button => {
                if self.in_scope(|tag| tag == Tag::Button, is_scope_boundary) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(|tag| tag == Tag::Button);
                }
                self.reconstruct_active_formatting();
                self.insert_element(token);
            }
            Tag::A => {
                if let Some(index) = self.active_formatting_index(Tag::A) {
                    let Active::Element(link) = self.active[index] else {
                        return;
                    };
                    self.adoption_agency(token);
                    self.active.retain(|&entry| entry != Active::Element(link));
                    if let Some(position) = self.open.iter().position(|&id| id == link) {
                        self.document.set_open(link, false);
                        self.open.remove(position);
                    }
                }
                self.insert_formatting(token);
            }
            Tag::Nobr => {
                self.reconstruct_active_formatting();
                if self.in_scope(|tag| tag == Tag::Nobr, is_scope_boundary) {
                    self.adoption_agency(token);
                }
                self.insert_formatting(token);
            }
            _ if tag.is_formatting() => self.insert_formatting(token),
            Tag::Applet | Tag::Marquee | Tag::Object => {
                self.reconstruct_active_formatting();
                self.insert_element(token);
                self.active.push(Active::Marker);
            }
            Tag::Table => {
                if matches!(
                    self.current_tag(),
                    Tag::Table | Tag::Tbody | Tag::Thead | Tag::Tfoot | Tag::Tr
                ) {
                    self.pop_until(|tag| tag == Tag::Table);
                }
                self.close_paragraph();
                self.insert_element(token);
            }
            Tag::Caption | Tag::Colgroup | Tag::Tbody | Tag::Thead | Tag::Tfoot => {
                if self.in_table() {
                    self.close_table_cell();
                    self.close_caption();
                    self.clear_to_table_context(|tag| tag == Tag::Table);
                    self.insert_element(token);
                    if tag == Tag::Caption {
                        self.active.push(Active::Marker);
                    }
                }
            }
            Tag::Col => {
                if self.in_table() {
                    self.close_table_cell();
                    self.clear_to_table_context(|tag| matches!(tag, Tag::Table | Tag::Colgroup));
                    if self.current_tag() == Tag::Table {
                        self.insert_implied(Tag::Colgroup);
                    }
                    self.insert_void(token);
                }
            }
            Tag::Tr => {
                if self.in_table() {
                    self.close_table_cell();
                    self.close_caption();
                    self.clear_to_table_context(is_table_section);
                    if self.current_tag() == Tag::Table {
                        self.insert_implied(Tag::Tbody);
                    }
                    self.insert_element(token);
                }
            }
            Tag::Td | Tag::Th => {
                if self.in_table() {
                    self.close_table_cell();
                    self.close_caption();
                    self.clear_to_table_context(|tag| tag == Tag::Tr || is_table_section(tag));
                    if self.current_tag() == Tag::Table {
                        self.insert_implied(Tag::Tbody);
                    }
                    if self.current_tag() != Tag::Tr {
                        self.insert_implied(Tag::Tr);
                    }
                    self.insert_element(token);
                    self.active.push(Active::Marker);
                }
            }
            Tag::Hr => {
                self.close_paragraph();
                self.insert_void(token);
            }
            _ if tag.is_void() => {
                self.reconstruct_active_formatting();
                self.insert_void(token);
            }
            Tag::Textarea => {
                self.insert_element(token);
                self.skip_newline = NewlineSkip::Next;
            }
            Tag::Xmp => {
                self.close_paragraph();
                self.reconstruct_active_formatting();
                self.insert_element(token);
            }
            Tag::Iframe | Tag::Noembed => {
                self.insert_element(token);
            }
            Tag::Option => {
                if self.current_tag() == Tag::Option {
                    self.pop();
                }
                self.reconstruct_active_formatting();
                self.insert_element(token);
            }
            Tag::Optgroup => {
                if self.current_tag() == Tag::Option {
                    self.pop();
                }
                if self.current_tag() == Tag::Optgroup {
                    self.pop();
                }
                self.reconstruct_active_formatting();
                self.insert_element(token);
            }
            Tag::Rb | Tag::Rtc | Tag::Rp | Tag::Rt => {
                if self.in_scope(
                    |tag| matches!(tag, Tag::Rb | Tag::Rtc | Tag::Rp | Tag::Rt),
                    is_scope_boundary,
                ) {
                    let keep = (tag != Tag::Rb && tag != Tag::Rtc).then_some(Tag::Rtc);
                    self.generate_implied_end_tags(keep);
                }
                self.insert_element(token);
            }
            _ if tag.closes_paragraph() => {
                self.close_paragraph();
                self.insert_element(token);
            }
            _ => {
                self.reconstruct_active_formatting();
                if token.self_closing && self.in_foreign_content(tag) {
                    self.insert_void(token);
                } else {
                    self.insert_element(token);
                }
            }
        }
    }

    fn end_tag_in_body(&mut self, token: TagToken) {
        let tag = token.tag;
        match tag {
            Tag::Body | Tag::Html => {
                if self.in_scope(|tag| tag == Tag::Body, is_scope_boundary) {
                    self.mode = Mode::AfterBody;
                }
            }
            Tag::Form => {
                let Some(form) = self.form.take() else {
                    return;
                };
                if self.in_scope(|tag| tag == Tag::Form, is_scope_boundary) {
                    self.generate_implied_end_tags(None);
                    if let Some(index) = self.open.iter().position(|&id| id == form) {
                        self.document.set_open(form, false);
                        self.open.remove(index);
                    }
                }
            }
            Tag::P => {
                if !self.in_scope(|tag| tag == Tag::P, is_button_scope_boundary) {
                    self.insert_implied(Tag::P);
                }
                self.close_paragraph();
            }
            Tag::Li => {
                if self.in_scope(|tag| tag == Tag::Li, is_list_scope_boundary) {
                    self.generate_implied_end_tags(Some(Tag::Li));
                    self.pop_until(|tag| tag == Tag::Li);
                }
            }
            Tag::Dd | Tag::Dt => {
                if self.in_scope(|other| other == tag, is_scope_boundary) {
                    self.generate_implied_end_tags(Some(tag));
                    self.pop_until(|other| other == tag);
                }
            }
            Tag::H1 | Tag::H2 | Tag::H3 | Tag::H4 | Tag::H5 | Tag::H6 => {
                if self.in_scope(Tag::is_heading, is_scope_boundary) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(Tag::is_heading);
                }
            }
            Tag::Applet | Tag::Marquee | Tag::Object => {
                if self.in_scope(|other| other == tag, is_scope_boundary) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(|other| other == tag);
                    self.clear_active_to_marker();
                }
            }
            Tag::Br => {
                self.reconstruct_active_formatting();
                let br = self
                    .document
                    .create(NodeKind::Element, Tag::Br, token.start..token.start);
                self.document.append(self.current(), br);
            }
            Tag::Td | Tag::Th => {
                if self.in_table_scope(|other| other == tag) {
                    self.close_table_cell();
                }
            }
            Tag::Tr => {
                if self.in_table_scope(|other| other == Tag::Tr) {
                    self.close_table_cell();
                    self.pop_until(|other| other == Tag::Tr);
                }
            }
            Tag::Tbody | Tag::Thead | Tag::Tfoot => {
                if self.in_table_scope(|other| other == tag) {
                    self.close_table_cell();
                    self.pop_until(is_table_section);
                }
            }
            Tag::Caption => self.close_caption(),
            Tag::Table => {
                if self.in_table() {
                    self.close_table_cell();
                    self.close_caption();
                    self.pop_until(|other| other == Tag::Table);
                }
            }
            Tag::Colgroup => {
                if self.current_tag() == Tag::Colgroup {
                    self.pop();
                }
            }
            _ if tag.closes_as_block() => {
                if self.in_scope(|other| other == tag, is_scope_boundary) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(|other| other == tag);
                }
            }
            _ if tag.is_formatting() => self.adoption_agency(token),
            _ => self.any_other_end_tag(token),
        }
    }

    fn any_other_end_tag(&mut self, token: TagToken) {
        for index in (0..self.open.len()).rev() {
            let id = self.open[index];
            if self.same_name(id, token) {
                self.generate_implied_end_tags(Some(token.tag));
                while self.open.len() > index {
                    self.pop();
                }
                return;
            }
            if self.document.tag(id).is_special() {
                return;
            }
        }
    }

    /// The adoption agency algorithm: repairs misnested formatting such as `<b><p></b>`.
    fn adoption_agency(&mut self, token: TagToken) {
        let subject = token.tag;
        if self.current_tag() == subject && !self.active.contains(&Active::Element(self.current()))
        {
            self.pop();
            return;
        }
        for _ in 0..8 {
            let Some(formatting_entry) = self.active_formatting_index(subject) else {
                self.any_other_end_tag(token);
                return;
            };
            let Active::Element(formatting) = self.active[formatting_entry] else {
                return;
            };
            let Some(formatting_index) = self.open.iter().position(|&id| id == formatting) else {
                self.active.remove(formatting_entry);
                return;
            };
            if !self.in_scope(|tag| tag == subject, is_scope_boundary) {
                return;
            }
            let furthest_index = (formatting_index + 1..self.open.len())
                .find(|&index| self.document.tag(self.open[index]).is_special());
            let Some(furthest_index) = furthest_index else {
                while self.open.len() > formatting_index {
                    self.pop();
                }
                self.active.remove(formatting_entry);
                return;
            };
            let furthest = self.open[furthest_index];
            let common_ancestor = self.open[formatting_index.saturating_sub(1)];
            let mut bookmark = formatting_entry;
            let mut last = furthest;
            let mut index = furthest_index;
            let mut inner = 0;
            loop {
                inner += 1;
                index -= 1;
                let node = self.open[index];
                if node == formatting {
                    break;
                }
                let entry = self.active.iter().position(|&e| e == Active::Element(node));
                let entry = match entry {
                    Some(entry) if inner > 3 => {
                        self.active.remove(entry);
                        if entry < bookmark {
                            bookmark -= 1;
                        }
                        None
                    }
                    other => other,
                };
                let Some(entry) = entry else {
                    self.document.set_open(node, false);
                    self.open.remove(index);
                    continue;
                };
                let clone = self.document.clone_element(node);
                self.document.set_open(node, false);
                self.document.set_open(clone, true);
                self.active[entry] = Active::Element(clone);
                self.open[index] = clone;
                if last == furthest {
                    bookmark = entry + 1;
                }
                self.document.detach(last);
                self.document.append(clone, last);
                last = clone;
            }
            self.document.detach(last);
            self.document.append(common_ancestor, last);
            let clone = self.document.clone_element(formatting);
            while let Some(child) = self.document.first_child(furthest) {
                self.document.detach(child);
                self.document.append(clone, child);
            }
            self.document.append(furthest, clone);
            let position = self
                .active
                .iter()
                .position(|&e| e == Active::Element(formatting));
            if let Some(position) = position {
                self.active.remove(position);
                if position < bookmark {
                    bookmark -= 1;
                }
            }
            self.active
                .insert(bookmark.min(self.active.len()), Active::Element(clone));
            let position = self.open.iter().position(|&id| id == formatting);
            if let Some(position) = position {
                self.document.set_open(formatting, false);
                self.open.remove(position);
            }
            let position = self.open.iter().position(|&id| id == furthest);
            let position = position.map_or(self.open.len(), |position| position + 1);
            self.document.set_open(clone, true);
            self.open.insert(position, clone);
        }
    }

    fn insert_formatting(&mut self, token: TagToken) {
        self.reconstruct_active_formatting();
        let element = self.insert_element(token);
        self.push_active(element);
    }

    /// Adds a formatting element; at most three identical ones stay active.
    fn push_active(&mut self, element: NodeId) {
        let text = self.document.text(element);
        let identical: Vec<usize> = (0..self.active.len())
            .rev()
            .map_while(|index| match self.active[index] {
                Active::Marker => None,
                Active::Element(other) => Some((index, other)),
            })
            .filter(|&(_, other)| {
                self.document.tag(other) == self.document.tag(element)
                    && self.document.text(other).eq_ignore_ascii_case(text)
            })
            .map(|(index, _)| index)
            .collect();
        if identical.len() >= 3 {
            self.active.remove(identical[identical.len() - 1]);
        }
        if self.active.len() >= MAX_ACTIVE_FORMATTING {
            self.active.remove(0);
        }
        self.active.push(Active::Element(element));
    }

    /// Reopens formatting elements that a closed block left unfinished.
    fn reconstruct_active_formatting(&mut self) {
        let is_settled = |parser: &Self, entry: Active| match entry {
            Active::Marker => true,
            Active::Element(id) => parser.document.is_open(id),
        };
        match self.active.last() {
            Some(&entry) if !is_settled(self, entry) => {}
            _ => return,
        }
        let mut first = self.active.len() - 1;
        while first > 0 && !is_settled(self, self.active[first - 1]) {
            first -= 1;
        }
        for index in first..self.active.len() {
            let Active::Element(element) = self.active[index] else {
                continue;
            };
            if self.open.len() >= MAX_DEPTH {
                self.pop();
            }
            let clone = self.document.clone_element(element);
            self.document.append(self.current(), clone);
            self.push_open(clone);
            self.active[index] = Active::Element(clone);
        }
    }

    fn active_formatting_index(&self, tag: Tag) -> Option<usize> {
        for index in (0..self.active.len()).rev() {
            match self.active[index] {
                Active::Marker => return None,
                Active::Element(id) if self.document.tag(id) == tag => return Some(index),
                Active::Element(_) => {}
            }
        }
        None
    }

    fn clear_active_to_marker(&mut self) {
        while let Some(entry) = self.active.pop() {
            if entry == Active::Marker {
                return;
            }
        }
    }

    fn close_paragraph(&mut self) {
        if self.in_scope(|tag| tag == Tag::P, is_button_scope_boundary) {
            self.generate_implied_end_tags(Some(Tag::P));
            self.pop_until(|tag| tag == Tag::P);
        }
    }

    /// Before a new list item: closes an open one, unless a special element separates them.
    fn close_list_item(&mut self, matches_item: impl Fn(Tag) -> bool) {
        for index in (0..self.open.len()).rev() {
            let tag = self.document.tag(self.open[index]);
            if matches_item(tag) {
                self.generate_implied_end_tags(Some(tag));
                self.pop_until(&matches_item);
                break;
            }
            if tag.is_special() && !matches!(tag, Tag::Address | Tag::Div | Tag::P) {
                break;
            }
        }
        self.close_paragraph();
    }

    fn in_table(&self) -> bool {
        self.in_table_scope(|tag| tag == Tag::Table)
    }

    fn in_table_scope(&self, target: impl Fn(Tag) -> bool) -> bool {
        self.in_scope(target, |tag| {
            matches!(tag, Tag::Html | Tag::Table | Tag::Template)
        })
    }

    fn close_table_cell(&mut self) {
        if self.in_table_scope(|tag| matches!(tag, Tag::Td | Tag::Th)) {
            self.generate_implied_end_tags(None);
            self.pop_until(|tag| matches!(tag, Tag::Td | Tag::Th));
            self.clear_active_to_marker();
        }
    }

    fn close_caption(&mut self) {
        if self.in_table_scope(|tag| tag == Tag::Caption) {
            self.generate_implied_end_tags(None);
            self.pop_until(|tag| tag == Tag::Caption);
            self.clear_active_to_marker();
        }
    }

    /// Closes elements until the current one fits `context` or is the table itself.
    fn clear_to_table_context(&mut self, context: impl Fn(Tag) -> bool) {
        while !context(self.current_tag())
            && !matches!(self.current_tag(), Tag::Table | Tag::Html)
            && !self.open.is_empty()
        {
            self.pop();
        }
    }

    fn in_scope(&self, target: impl Fn(Tag) -> bool, boundary: impl Fn(Tag) -> bool) -> bool {
        for &id in self.open.iter().rev() {
            let tag = self.document.tag(id);
            if target(tag) {
                return true;
            }
            if boundary(tag) {
                return false;
            }
        }
        false
    }

    fn in_foreign_content(&self, tag: Tag) -> bool {
        matches!(tag, Tag::Svg | Tag::Math)
            || self
                .open
                .iter()
                .any(|&id| matches!(self.document.tag(id), Tag::Svg | Tag::Math))
    }

    fn generate_implied_end_tags(&mut self, except: Option<Tag>) {
        loop {
            let tag = self.current_tag();
            if !tag.has_implied_end() || Some(tag) == except {
                return;
            }
            self.pop();
        }
    }

    fn same_name(&self, id: NodeId, token: TagToken) -> bool {
        let tag = self.document.tag(id);
        if tag != token.tag {
            return false;
        }
        tag != Tag::Other
            || self
                .document
                .name(id)
                .eq_ignore_ascii_case(&self.document.source[token.name_start..token.name_end])
    }

    fn current(&self) -> NodeId {
        self.open.last().copied().unwrap_or(self.document.root())
    }

    fn current_tag(&self) -> Tag {
        self.document.tag(self.current())
    }

    fn push_open(&mut self, id: NodeId) {
        self.document.set_open(id, true);
        self.open.push(id);
    }

    fn pop(&mut self) {
        if let Some(id) = self.open.pop() {
            self.document.set_open(id, false);
        }
    }

    fn pop_until(&mut self, target: impl Fn(Tag) -> bool) {
        while let Some(id) = self.open.pop() {
            self.document.set_open(id, false);
            if target(self.document.tag(id)) {
                return;
            }
        }
    }

    fn insert_element(&mut self, token: TagToken) -> NodeId {
        self.insert_open(token.tag, token.span())
    }

    /// Creates an element the source left out; it has an empty span and no attributes.
    fn insert_implied(&mut self, tag: Tag) -> NodeId {
        let end = self.document.source.len();
        self.insert_open(tag, end..end)
    }

    fn insert_open(&mut self, tag: Tag, span: Range<usize>) -> NodeId {
        if self.open.len() >= MAX_DEPTH {
            self.pop();
        }
        let element = self.document.create(NodeKind::Element, tag, span);
        self.document.append(self.current(), element);
        self.push_open(element);
        element
    }

    fn insert_void(&mut self, token: TagToken) {
        let element = self
            .document
            .create(NodeKind::Element, token.tag, token.span());
        self.document.append(self.current(), element);
    }

    fn insert_text(&mut self, span: Range<usize>) {
        let parent = self.current();
        if let Some(last) = self.document.last_child(parent)
            && self.document.extend_text(last, &span)
        {
            return;
        }
        let text = self.document.create(NodeKind::Text, Tag::Other, span);
        self.document.append(parent, text);
    }

    fn insert_comment(&mut self, span: Range<usize>) {
        let comment = self.document.create(NodeKind::Comment, Tag::Other, span);
        self.document.append(self.current(), comment);
    }
}

fn is_scope_boundary(tag: Tag) -> bool {
    matches!(
        tag,
        Tag::Applet
            | Tag::Caption
            | Tag::Html
            | Tag::Marquee
            | Tag::Object
            | Tag::Table
            | Tag::Td
            | Tag::Template
            | Tag::Th
    )
}

fn is_button_scope_boundary(tag: Tag) -> bool {
    tag == Tag::Button || is_scope_boundary(tag)
}

fn is_list_scope_boundary(tag: Tag) -> bool {
    matches!(tag, Tag::Ol | Tag::Ul) || is_scope_boundary(tag)
}

fn is_table_section(tag: Tag) -> bool {
    matches!(tag, Tag::Tbody | Tag::Thead | Tag::Tfoot)
}

/// Parses a complete document.
#[must_use]
pub fn parse(source: &str) -> Document {
    let mut parser = Parser::with_capacity(source.len());
    parser.feed(source);
    parser.finish();
    parser.into_document()
}
