//! Builds the document tree from tokens, one at a time, following the insertion modes of
//! the HTML standard. Scripting counts as disabled and scripts do not run. Attribute
//! names in SVG and MathML keep the form they were written in.

use std::ops::Range;

use super::attributes::{attributes, is_closed_tag};
use super::dom::{Document, Namespace, NodeId, NodeKind};
use super::tag::Tag;
use super::tokenizer::{Scanner, Token, TokenKind, is_html_space, tag_name};

/// Limit of open elements; further ones become siblings. Searches walk the open elements,
/// so this bounds them. The tree can get deeper, so code walking it must not recurse.
const MAX_DEPTH: usize = 512;
/// The oldest entries are dropped beyond this; it bounds the elements one text can reopen.
const MAX_ACTIVE_FORMATTING: usize = 64;
/// The tree stops growing here, so that small hostile input cannot exhaust memory.
const MAX_NODES: usize = 1 << 24;
/// Cloned elements allowed before any input, then per input byte. Ordinary pages clone
/// almost nothing, while hostile ones could otherwise reach a hundred times their size.
const FREE_CLONES: usize = 1000;
const INPUT_BYTES_PER_CLONE: usize = 4;
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
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
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

/// What the tree builder receives. Text spans can shrink as modes take off their share.
#[derive(Clone, Copy)]
enum Input {
    Text(usize, usize),
    Comment(usize, usize),
    Doctype(usize, usize),
    Start(TagToken),
    End(TagToken),
    EndOfFile,
}

/// Parses a document that arrives in pieces. After each [`Parser::feed`] the tree holds
/// everything that can already be known and may be shown. Tokens are not kept.
pub struct Parser {
    document: Document,
    scanner: Scanner,
    finished: bool,
    truncated: bool,
    /// Bytes of input processed so far, which does not depend on the sizes of the pieces.
    processed: usize,
    clones: usize,
    formatting_limited: bool,
    mode: Mode,
    /// The mode to return to after [`Mode::Text`] or [`Mode::InTableText`].
    original_mode: Mode,
    template_modes: Vec<Mode>,
    open: Vec<NodeId>,
    active: Vec<Active>,
    head: Option<NodeId>,
    form: Option<NodeId>,
    frameset_ok: bool,
    /// Misplaced table content goes in front of the table.
    foster: bool,
    pending_table_text: Vec<(usize, usize)>,
    pending_table_text_has_content: bool,
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
            scanner: Scanner::guided(),
            finished: false,
            truncated: false,
            processed: 0,
            clones: 0,
            formatting_limited: false,
            mode: Mode::Initial,
            original_mode: Mode::Initial,
            template_modes: Vec::new(),
            open: Vec::new(),
            active: Vec::new(),
            head: None,
            form: None,
            frameset_ok: true,
            foster: false,
            pending_table_text: Vec::new(),
            pending_table_text_has_content: false,
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
        self.dispatch(Input::EndOfFile);
        self.document.shrink_to_fit();
        self.open = Vec::new();
        self.active = Vec::new();
        self.template_modes = Vec::new();
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

    /// Whether the clone budget ran out, so that some text lost its formatting.
    #[must_use]
    pub fn is_formatting_limited(&self) -> bool {
        self.formatting_limited
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
        self.processed = span.end;
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
            TokenKind::RawText => {
                // Outside raw-text elements this comes from `<plaintext>` or a CDATA section.
                if self.mode == Mode::Text {
                    self.insert_text(span.start, span.end);
                } else {
                    self.dispatch(Input::Text(span.start, span.end));
                }
            }
            TokenKind::Text | TokenKind::StrayLt => {
                self.dispatch(Input::Text(span.start, span.end));
            }
            TokenKind::Comment => self.dispatch(Input::Comment(span.start, span.end)),
            TokenKind::Declaration => self.process_declaration(span),
            // A tag that the input cut off does not count.
            TokenKind::StartTag | TokenKind::EndTag
                if self.finished
                    && span.end == self.document.source.len()
                    && !is_closed_tag(&self.document.source[span.clone()]) => {}
            TokenKind::StartTag => {
                let tag = self.tag_token(span);
                self.dispatch(Input::Start(tag));
            }
            TokenKind::EndTag => {
                let tag = self.tag_token(span);
                self.dispatch(Input::End(tag));
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

    fn process_declaration(&mut self, span: Range<usize>) {
        let text = &self.document.source[span.clone()];
        if text == "</>" {
            return;
        }
        if text.starts_with("<![CDATA[")
            && !self.open.is_empty()
            && self.document.namespace(self.current()) != Namespace::Html
        {
            self.scanner.enter_cdata(span.start + 9);
            return;
        }
        let is_doctype = text
            .get(..9)
            .is_some_and(|start| start.eq_ignore_ascii_case("<!doctype"));
        if is_doctype {
            self.dispatch(Input::Doctype(span.start, span.end));
        } else {
            self.dispatch(Input::Comment(span.start, span.end));
        }
    }

    fn dispatch(&mut self, input: Input) {
        if self.uses_foreign_rules(input) {
            self.in_foreign_content(input);
        } else {
            self.by_mode(self.mode, input);
        }
    }

    fn uses_foreign_rules(&self, input: Input) -> bool {
        if self.open.is_empty() || matches!(input, Input::EndOfFile) {
            return false;
        }
        let current = self.current();
        if self.document.namespace(current) == Namespace::Html {
            return false;
        }
        let tag = self.document.tag(current);
        let is_start = matches!(input, Input::Start(_));
        let is_content = is_start || matches!(input, Input::Text(..));
        if tag.is_math_text_integration_point() {
            let math_child = match input {
                Input::Start(token) => {
                    let name = self.token_name(token);
                    name.eq_ignore_ascii_case("mglyph") || name.eq_ignore_ascii_case("malignmark")
                }
                _ => false,
            };
            if is_content && !math_child {
                return false;
            }
        }
        if tag == Tag::MathAnnotationXml
            && let Input::Start(token) = input
            && token.tag == Tag::Svg
        {
            return false;
        }
        !(tag.is_html_integration_point() && is_content)
    }

    fn by_mode(&mut self, mode: Mode, input: Input) {
        match mode {
            Mode::Initial => self.initial(input),
            Mode::BeforeHtml => self.before_html(input),
            Mode::BeforeHead => self.before_head(input),
            Mode::InHead => self.in_head(input),
            Mode::InHeadNoscript => self.in_head_noscript(input),
            Mode::AfterHead => self.after_head(input),
            Mode::InBody => self.in_body(input),
            Mode::Text => self.in_text(input),
            Mode::InTable => self.in_table(input),
            Mode::InTableText => self.in_table_text(input),
            Mode::InCaption => self.in_caption(input),
            Mode::InColumnGroup => self.in_column_group(input),
            Mode::InTableBody => self.in_table_body(input),
            Mode::InRow => self.in_row(input),
            Mode::InCell => self.in_cell(input),
            Mode::InTemplate => self.in_template(input),
            Mode::AfterBody => self.after_body(input),
            Mode::InFrameset => self.in_frameset(input),
            Mode::AfterFrameset => self.after_frameset(input),
            Mode::AfterAfterBody => self.after_after_body(input),
            Mode::AfterAfterFrameset => self.after_after_frameset(input),
        }
    }

    fn switch(&mut self, mode: Mode, input: Input) {
        self.mode = mode;
        self.dispatch(input);
    }

    fn initial(&mut self, input: Input) {
        match input {
            Input::Text(start, end) => {
                let start = start + self.leading_space(start, end);
                if start < end {
                    self.document.set_quirks(true);
                    self.switch(Mode::BeforeHtml, Input::Text(start, end));
                }
            }
            Input::Comment(start, end) => self.append_comment(self.document.root(), start, end),
            Input::Doctype(start, end) => {
                let doctype = self
                    .document
                    .create(NodeKind::Doctype, Tag::Other, start..end);
                self.document.append(self.document.root(), doctype);
                let quirks = self.document.doctype(doctype).is_quirks();
                self.document.set_quirks(quirks);
                self.mode = Mode::BeforeHtml;
            }
            _ => {
                self.document.set_quirks(true);
                self.switch(Mode::BeforeHtml, input);
            }
        }
    }

    fn before_html(&mut self, input: Input) {
        match input {
            Input::Doctype(..) => {}
            Input::Comment(start, end) => self.append_comment(self.document.root(), start, end),
            Input::Text(start, end) => {
                let start = start + self.leading_space(start, end);
                if start < end {
                    self.insert_implied(Tag::Html);
                    self.switch(Mode::BeforeHead, Input::Text(start, end));
                }
            }
            Input::Start(token) if token.tag == Tag::Html => {
                self.insert_html(token);
                self.mode = Mode::BeforeHead;
            }
            Input::End(token)
                if !matches!(token.tag, Tag::Head | Tag::Body | Tag::Html | Tag::Br) => {}
            _ => {
                self.insert_implied(Tag::Html);
                self.switch(Mode::BeforeHead, input);
            }
        }
    }

    fn before_head(&mut self, input: Input) {
        match input {
            Input::Text(start, end) => {
                let start = start + self.leading_space(start, end);
                if start < end {
                    self.head = Some(self.insert_implied(Tag::Head));
                    self.switch(Mode::InHead, Input::Text(start, end));
                }
            }
            Input::Comment(start, end) => self.insert_comment(start, end),
            Input::Doctype(..) => {}
            Input::Start(token) if token.tag == Tag::Html => self.in_body(input),
            Input::Start(token) if token.tag == Tag::Head => {
                self.head = Some(self.insert_html(token));
                self.mode = Mode::InHead;
            }
            Input::End(token)
                if !matches!(token.tag, Tag::Head | Tag::Body | Tag::Html | Tag::Br) => {}
            _ => {
                self.head = Some(self.insert_implied(Tag::Head));
                self.switch(Mode::InHead, input);
            }
        }
    }

    fn in_head(&mut self, input: Input) {
        match input {
            Input::Text(start, end) => {
                let space = self.leading_space(start, end);
                if space > 0 {
                    self.insert_text(start, start + space);
                }
                if start + space < end {
                    self.pop();
                    self.switch(Mode::AfterHead, Input::Text(start + space, end));
                }
            }
            Input::Comment(start, end) => self.insert_comment(start, end),
            Input::Doctype(..) => {}
            Input::Start(token) => match token.tag {
                Tag::Html => self.in_body(input),
                Tag::Base | Tag::Basefont | Tag::Bgsound | Tag::Link | Tag::Meta => {
                    self.insert_void(token);
                }
                Tag::Title => self.insert_raw(token),
                Tag::Noscript => {
                    self.insert_html(token);
                    self.mode = Mode::InHeadNoscript;
                }
                Tag::Noframes | Tag::Style => self.insert_raw(token),
                Tag::Script => {
                    self.insert_html(token);
                    self.scanner.enter_script();
                    self.original_mode = self.mode;
                    self.mode = Mode::Text;
                }
                Tag::Template => {
                    self.insert_html(token);
                    self.active.push(Active::Marker);
                    self.frameset_ok = false;
                    self.mode = Mode::InTemplate;
                    self.template_modes.push(Mode::InTemplate);
                }
                Tag::Head => {}
                _ => {
                    self.pop();
                    self.switch(Mode::AfterHead, input);
                }
            },
            Input::End(token) => match token.tag {
                Tag::Head => {
                    self.pop();
                    self.mode = Mode::AfterHead;
                }
                Tag::Body | Tag::Html | Tag::Br => {
                    self.pop();
                    self.switch(Mode::AfterHead, input);
                }
                Tag::Template if self.has_open(Tag::Template) => {
                    self.generate_implied_end_tags_thoroughly();
                    self.pop_until(|tag| tag == Tag::Template);
                    self.clear_active_to_marker();
                    self.template_modes.pop();
                    self.reset_mode();
                }
                _ => {}
            },
            Input::EndOfFile => {
                self.pop();
                self.switch(Mode::AfterHead, input);
            }
        }
    }

    fn in_head_noscript(&mut self, input: Input) {
        match input {
            Input::Doctype(..) => {}
            Input::Start(token) if token.tag == Tag::Html => self.in_body(input),
            Input::End(token) if token.tag == Tag::Noscript => {
                self.pop();
                self.mode = Mode::InHead;
            }
            Input::Comment(..) => self.in_head(input),
            Input::Text(start, end) if self.leading_space(start, end) > 0 => {
                let space = self.leading_space(start, end);
                self.insert_text(start, start + space);
                if start + space < end {
                    self.in_head_noscript(Input::Text(start + space, end));
                }
            }
            Input::Start(token)
                if matches!(
                    token.tag,
                    Tag::Basefont
                        | Tag::Bgsound
                        | Tag::Link
                        | Tag::Meta
                        | Tag::Noframes
                        | Tag::Style
                ) =>
            {
                self.in_head(input);
            }
            Input::Start(token) if matches!(token.tag, Tag::Head | Tag::Noscript) => {}
            Input::End(token) if token.tag != Tag::Br => {}
            _ => {
                self.pop();
                self.switch(Mode::InHead, input);
            }
        }
    }

    fn after_head(&mut self, input: Input) {
        match input {
            Input::Text(start, end) => {
                let space = self.leading_space(start, end);
                if space > 0 {
                    self.insert_text(start, start + space);
                }
                if start + space < end {
                    self.insert_implied(Tag::Body);
                    self.switch(Mode::InBody, Input::Text(start + space, end));
                }
            }
            Input::Comment(start, end) => self.insert_comment(start, end),
            Input::Doctype(..) => {}
            Input::Start(token) => match token.tag {
                Tag::Html => self.in_body(input),
                Tag::Body => {
                    self.insert_html(token);
                    self.frameset_ok = false;
                    self.mode = Mode::InBody;
                }
                Tag::Frameset => {
                    self.insert_html(token);
                    self.mode = Mode::InFrameset;
                }
                Tag::Base
                | Tag::Basefont
                | Tag::Bgsound
                | Tag::Link
                | Tag::Meta
                | Tag::Noframes
                | Tag::Script
                | Tag::Style
                | Tag::Template
                | Tag::Title => {
                    // Late head content still goes into the head.
                    let Some(head) = self.head else {
                        return;
                    };
                    self.push_open(head);
                    self.in_head(input);
                    if let Some(index) = self.open.iter().position(|&id| id == head) {
                        self.document.set_open(head, false);
                        self.open.remove(index);
                    }
                }
                Tag::Head => {}
                _ => {
                    self.insert_implied(Tag::Body);
                    self.switch(Mode::InBody, input);
                }
            },
            Input::End(token) => match token.tag {
                Tag::Template => self.in_head(input),
                Tag::Body | Tag::Html | Tag::Br => {
                    self.insert_implied(Tag::Body);
                    self.switch(Mode::InBody, input);
                }
                _ => {}
            },
            Input::EndOfFile => {
                self.insert_implied(Tag::Body);
                self.switch(Mode::InBody, input);
            }
        }
    }

    fn in_body(&mut self, input: Input) {
        match input {
            Input::Text(start, end) => {
                if self.has_content(start, end) {
                    self.frameset_ok = false;
                }
                self.reconstruct_active_formatting();
                self.insert_text(start, end);
            }
            Input::Comment(start, end) => self.insert_comment(start, end),
            Input::Doctype(..) => {}
            Input::Start(token) => self.start_tag_in_body(token),
            Input::End(token) => self.end_tag_in_body(token),
            Input::EndOfFile => {
                if !self.template_modes.is_empty() {
                    self.in_template(input);
                }
            }
        }
    }

    fn start_tag_in_body(&mut self, token: TagToken) {
        let tag = token.tag;
        match tag {
            Tag::Html => {
                if !self.has_open(Tag::Template) && !self.open.is_empty() {
                    self.merge_attributes(self.open[0], token);
                }
            }
            Tag::Base
            | Tag::Basefont
            | Tag::Bgsound
            | Tag::Link
            | Tag::Meta
            | Tag::Noframes
            | Tag::Script
            | Tag::Style
            | Tag::Template
            | Tag::Title => self.in_head(Input::Start(token)),
            Tag::Body => {
                let body = self
                    .open
                    .get(1)
                    .copied()
                    .filter(|&id| self.document.tag(id) == Tag::Body);
                if let Some(body) = body
                    && !self.has_open(Tag::Template)
                {
                    self.frameset_ok = false;
                    self.merge_attributes(body, token);
                }
            }
            Tag::Frameset => {
                let body = self
                    .open
                    .get(1)
                    .copied()
                    .filter(|&id| self.document.tag(id) == Tag::Body);
                if let Some(body) = body
                    && self.frameset_ok
                {
                    self.document.detach(body);
                    while self.open.len() > 1 {
                        self.pop();
                    }
                    self.insert_html(token);
                    self.mode = Mode::InFrameset;
                }
            }
            Tag::H1 | Tag::H2 | Tag::H3 | Tag::H4 | Tag::H5 | Tag::H6 => {
                self.close_paragraph();
                if self.current_tag().is_heading() {
                    self.pop();
                }
                self.insert_html(token);
            }
            Tag::Pre | Tag::Listing => {
                self.close_paragraph();
                self.insert_html(token);
                self.skip_newline = NewlineSkip::Next;
                self.frameset_ok = false;
            }
            Tag::Form => {
                let in_template = self.has_open(Tag::Template);
                if self.form.is_none() || in_template {
                    self.close_paragraph();
                    let form = self.insert_html(token);
                    if !in_template {
                        self.form = Some(form);
                    }
                }
            }
            Tag::Li => {
                self.frameset_ok = false;
                self.close_list_item(|tag| tag == Tag::Li);
                self.insert_html(token);
            }
            Tag::Dd | Tag::Dt => {
                self.frameset_ok = false;
                self.close_list_item(|tag| matches!(tag, Tag::Dd | Tag::Dt));
                self.insert_html(token);
            }
            Tag::Plaintext => {
                self.close_paragraph();
                self.insert_html(token);
                self.scanner.enter_plain_text();
            }
            Tag::Button => {
                if self.in_scope(|tag| tag == Tag::Button, is_scope_boundary) {
                    self.generate_implied_end_tags(None);
                    self.pop_until(|tag| tag == Tag::Button);
                }
                self.reconstruct_active_formatting();
                self.insert_html(token);
                self.frameset_ok = false;
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
                self.insert_html(token);
                self.active.push(Active::Marker);
                self.frameset_ok = false;
            }
            Tag::Table => {
                if !self.document.is_quirks() {
                    self.close_paragraph();
                }
                self.insert_html(token);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            }
            Tag::Area | Tag::Br | Tag::Embed | Tag::Img | Tag::Keygen | Tag::Wbr => {
                self.reconstruct_active_formatting();
                self.insert_void(token);
                self.frameset_ok = false;
            }
            Tag::Image => {
                let token = TagToken {
                    tag: Tag::Img,
                    ..token
                };
                self.start_tag_in_body(token);
            }
            Tag::Input => {
                if self.in_scope(|tag| tag == Tag::Select, is_scope_boundary) {
                    self.pop_until(|tag| tag == Tag::Select);
                }
                self.reconstruct_active_formatting();
                self.insert_void(token);
                if !self.is_hidden_input(token) {
                    self.frameset_ok = false;
                }
            }
            Tag::Param | Tag::Source | Tag::Track => self.insert_void(token),
            Tag::Hr => {
                self.close_paragraph();
                if self.in_scope(|tag| tag == Tag::Select, is_scope_boundary) {
                    self.generate_implied_end_tags(None);
                }
                self.insert_void(token);
                self.frameset_ok = false;
            }
            Tag::Textarea => {
                self.insert_raw(token);
                self.skip_newline = NewlineSkip::Next;
                self.frameset_ok = false;
            }
            Tag::Xmp => {
                self.close_paragraph();
                self.reconstruct_active_formatting();
                self.frameset_ok = false;
                self.insert_raw(token);
            }
            Tag::Iframe => {
                self.frameset_ok = false;
                self.insert_raw(token);
            }
            Tag::Noembed => self.insert_raw(token),
            Tag::Select => {
                if self.in_scope(|tag| tag == Tag::Select, is_scope_boundary) {
                    self.pop_until(|tag| tag == Tag::Select);
                } else {
                    self.reconstruct_active_formatting();
                    self.insert_html(token);
                    self.frameset_ok = false;
                }
            }
            Tag::Option => {
                if self.in_scope(|tag| tag == Tag::Select, is_scope_boundary) {
                    self.generate_implied_end_tags(Some(Tag::Optgroup));
                } else if self.current_tag() == Tag::Option {
                    self.pop();
                }
                self.reconstruct_active_formatting();
                self.insert_html(token);
            }
            Tag::Optgroup => {
                if self.in_scope(|tag| tag == Tag::Select, is_scope_boundary) {
                    self.generate_implied_end_tags(None);
                } else if self.current_tag() == Tag::Option {
                    self.pop();
                }
                self.reconstruct_active_formatting();
                self.insert_html(token);
            }
            Tag::Rb | Tag::Rtc => {
                if self.in_scope(|tag| tag == Tag::Ruby, is_scope_boundary) {
                    self.generate_implied_end_tags(None);
                }
                self.insert_html(token);
            }
            Tag::Rp | Tag::Rt => {
                if self.in_scope(|tag| tag == Tag::Ruby, is_scope_boundary) {
                    self.generate_implied_end_tags(Some(Tag::Rtc));
                }
                self.insert_html(token);
            }
            Tag::Math => self.start_foreign_root(token, Namespace::MathMl),
            Tag::Svg => self.start_foreign_root(token, Namespace::Svg),
            Tag::Caption
            | Tag::Col
            | Tag::Colgroup
            | Tag::Frame
            | Tag::Head
            | Tag::Tbody
            | Tag::Td
            | Tag::Tfoot
            | Tag::Th
            | Tag::Thead
            | Tag::Tr => {}
            _ if tag.closes_paragraph() => {
                self.close_paragraph();
                self.insert_html(token);
            }
            _ => {
                self.reconstruct_active_formatting();
                self.insert_html(token);
            }
        }
    }

    fn start_foreign_root(&mut self, token: TagToken, namespace: Namespace) {
        self.reconstruct_active_formatting();
        self.insert_foreign(token, namespace);
    }

    fn end_tag_in_body(&mut self, token: TagToken) {
        let tag = token.tag;
        match tag {
            Tag::Template => self.in_head(Input::End(token)),
            Tag::Body | Tag::Html => {
                if self.in_scope(|tag| tag == Tag::Body, is_scope_boundary) {
                    self.mode = Mode::AfterBody;
                    if tag == Tag::Html {
                        self.dispatch(Input::End(token));
                    }
                }
            }
            Tag::Form => self.end_form(),
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
                self.insert_node(br);
                self.frameset_ok = false;
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

    fn end_form(&mut self) {
        if self.has_open(Tag::Template) {
            if self.in_scope(|tag| tag == Tag::Form, is_scope_boundary) {
                self.generate_implied_end_tags(None);
                self.pop_until(|tag| tag == Tag::Form);
            }
            return;
        }
        let Some(form) = self.form.take() else {
            return;
        };
        for &id in self.open.iter().rev() {
            if id == form {
                break;
            }
            if is_scope_boundary(self.document.tag(id)) {
                return;
            }
        }
        self.generate_implied_end_tags(None);
        if let Some(index) = self.open.iter().position(|&id| id == form) {
            self.document.set_open(form, false);
            self.open.remove(index);
        }
    }

    fn in_text(&mut self, input: Input) {
        match input {
            Input::End(_) => {
                self.pop();
                self.mode = self.original_mode;
            }
            Input::EndOfFile => {
                self.pop();
                self.switch(self.original_mode, input);
            }
            _ => {}
        }
    }

    fn in_table(&mut self, input: Input) {
        match input {
            Input::Text(..)
                if matches!(
                    self.current_tag(),
                    Tag::Table | Tag::Tbody | Tag::Template | Tag::Tfoot | Tag::Thead | Tag::Tr
                ) =>
            {
                self.pending_table_text.clear();
                self.pending_table_text_has_content = false;
                self.original_mode = self.mode;
                self.switch(Mode::InTableText, input);
            }
            Input::Comment(start, end) => self.insert_comment(start, end),
            Input::Doctype(..) => {}
            Input::Start(token) => match token.tag {
                Tag::Caption => {
                    self.clear_to_table_context();
                    self.active.push(Active::Marker);
                    self.insert_html(token);
                    self.mode = Mode::InCaption;
                }
                Tag::Colgroup => {
                    self.clear_to_table_context();
                    self.insert_html(token);
                    self.mode = Mode::InColumnGroup;
                }
                Tag::Col => {
                    self.clear_to_table_context();
                    self.insert_implied(Tag::Colgroup);
                    self.switch(Mode::InColumnGroup, input);
                }
                Tag::Tbody | Tag::Tfoot | Tag::Thead => {
                    self.clear_to_table_context();
                    self.insert_html(token);
                    self.mode = Mode::InTableBody;
                }
                Tag::Td | Tag::Th | Tag::Tr => {
                    self.clear_to_table_context();
                    self.insert_implied(Tag::Tbody);
                    self.switch(Mode::InTableBody, input);
                }
                Tag::Table => {
                    if self.in_table_scope(|tag| tag == Tag::Table) {
                        self.pop_until(|tag| tag == Tag::Table);
                        self.reset_mode();
                        self.dispatch(input);
                    }
                }
                Tag::Style | Tag::Script | Tag::Template => self.in_head(input),
                Tag::Input if self.is_hidden_input(token) => self.insert_void(token),
                Tag::Form => {
                    if self.form.is_none() && !self.has_open(Tag::Template) {
                        let form = self.insert_html(token);
                        self.form = Some(form);
                        self.pop();
                    }
                }
                _ => self.fostered_in_body(input),
            },
            Input::End(token) => match token.tag {
                Tag::Table => {
                    if self.in_table_scope(|tag| tag == Tag::Table) {
                        self.pop_until(|tag| tag == Tag::Table);
                        self.reset_mode();
                    }
                }
                Tag::Body
                | Tag::Caption
                | Tag::Col
                | Tag::Colgroup
                | Tag::Html
                | Tag::Tbody
                | Tag::Td
                | Tag::Tfoot
                | Tag::Th
                | Tag::Thead
                | Tag::Tr => {}
                Tag::Template => self.in_head(input),
                _ => self.fostered_in_body(input),
            },
            Input::EndOfFile => self.in_body(input),
            Input::Text(..) => self.fostered_in_body(input),
        }
    }

    fn fostered_in_body(&mut self, input: Input) {
        self.foster = true;
        self.in_body(input);
        self.foster = false;
    }

    fn in_table_text(&mut self, input: Input) {
        if let Input::Text(start, end) = input {
            self.pending_table_text.push((start, end));
            self.pending_table_text_has_content |= self.has_content(start, end);
            return;
        }
        let pending = std::mem::take(&mut self.pending_table_text);
        for &(start, end) in &pending {
            if self.pending_table_text_has_content {
                self.fostered_in_body(Input::Text(start, end));
            } else {
                self.insert_text(start, end);
            }
        }
        self.pending_table_text = pending;
        self.pending_table_text.clear();
        self.switch(self.original_mode, input);
    }

    fn close_caption(&mut self) -> bool {
        if !self.in_table_scope(|tag| tag == Tag::Caption) {
            return false;
        }
        self.generate_implied_end_tags(None);
        self.pop_until(|tag| tag == Tag::Caption);
        self.clear_active_to_marker();
        self.mode = Mode::InTable;
        true
    }

    fn in_caption(&mut self, input: Input) {
        match input {
            Input::End(token) if token.tag == Tag::Caption => {
                self.close_caption();
            }
            Input::Start(token)
                if matches!(
                    token.tag,
                    Tag::Caption
                        | Tag::Col
                        | Tag::Colgroup
                        | Tag::Tbody
                        | Tag::Td
                        | Tag::Tfoot
                        | Tag::Th
                        | Tag::Thead
                        | Tag::Tr
                ) =>
            {
                if self.close_caption() {
                    self.dispatch(input);
                }
            }
            Input::End(token) if token.tag == Tag::Table => {
                if self.close_caption() {
                    self.dispatch(input);
                }
            }
            Input::End(token)
                if matches!(
                    token.tag,
                    Tag::Body
                        | Tag::Col
                        | Tag::Colgroup
                        | Tag::Html
                        | Tag::Tbody
                        | Tag::Td
                        | Tag::Tfoot
                        | Tag::Th
                        | Tag::Thead
                        | Tag::Tr
                ) => {}
            _ => self.in_body(input),
        }
    }

    fn in_column_group(&mut self, input: Input) {
        match input {
            Input::Text(start, end) if self.leading_space(start, end) > 0 => {
                let space = self.leading_space(start, end);
                self.insert_text(start, start + space);
                if start + space < end {
                    self.in_column_group(Input::Text(start + space, end));
                }
            }
            Input::Comment(start, end) => self.insert_comment(start, end),
            Input::Doctype(..) => {}
            Input::Start(token) if token.tag == Tag::Html => self.in_body(input),
            Input::Start(token) if token.tag == Tag::Col => self.insert_void(token),
            Input::End(token) if token.tag == Tag::Colgroup => {
                if self.current_tag() == Tag::Colgroup {
                    self.pop();
                    self.mode = Mode::InTable;
                }
            }
            Input::End(token) if token.tag == Tag::Col => {}
            Input::Start(token) if token.tag == Tag::Template => self.in_head(input),
            Input::End(token) if token.tag == Tag::Template => self.in_head(input),
            Input::EndOfFile => self.in_body(input),
            _ => {
                if self.current_tag() == Tag::Colgroup {
                    self.pop();
                    self.switch(Mode::InTable, input);
                }
            }
        }
    }

    fn clear_to_table_body_context(&mut self) {
        self.clear_to(|tag| {
            matches!(
                tag,
                Tag::Tbody | Tag::Tfoot | Tag::Thead | Tag::Template | Tag::Html
            )
        });
    }

    fn clear_to_table_row_context(&mut self) {
        self.clear_to(|tag| matches!(tag, Tag::Tr | Tag::Template | Tag::Html));
    }

    fn clear_to_table_context(&mut self) {
        self.clear_to(|tag| matches!(tag, Tag::Table | Tag::Template | Tag::Html));
    }

    fn clear_to(&mut self, stop: impl Fn(Tag) -> bool) {
        while !self.open.is_empty() && !stop(self.current_tag()) {
            self.pop();
        }
    }

    fn in_table_body(&mut self, input: Input) {
        match input {
            Input::Start(token) if token.tag == Tag::Tr => {
                self.clear_to_table_body_context();
                self.insert_html(token);
                self.mode = Mode::InRow;
            }
            Input::Start(token) if matches!(token.tag, Tag::Th | Tag::Td) => {
                self.clear_to_table_body_context();
                self.insert_implied(Tag::Tr);
                self.switch(Mode::InRow, input);
            }
            Input::End(token) if is_table_section(token.tag) => {
                if self.in_table_scope(|tag| tag == token.tag) {
                    self.clear_to_table_body_context();
                    self.pop();
                    self.mode = Mode::InTable;
                }
            }
            Input::Start(token)
                if matches!(token.tag, Tag::Caption | Tag::Col | Tag::Colgroup)
                    || is_table_section(token.tag) =>
            {
                self.leave_table_body(input);
            }
            Input::End(token) if token.tag == Tag::Table => self.leave_table_body(input),
            Input::End(token)
                if matches!(
                    token.tag,
                    Tag::Body
                        | Tag::Caption
                        | Tag::Col
                        | Tag::Colgroup
                        | Tag::Html
                        | Tag::Td
                        | Tag::Th
                        | Tag::Tr
                ) => {}
            _ => self.in_table(input),
        }
    }

    fn leave_table_body(&mut self, input: Input) {
        if self.in_table_scope(is_table_section) {
            self.clear_to_table_body_context();
            self.pop();
            self.switch(Mode::InTable, input);
        }
    }

    fn in_row(&mut self, input: Input) {
        match input {
            Input::Start(token) if matches!(token.tag, Tag::Th | Tag::Td) => {
                self.clear_to_table_row_context();
                self.insert_html(token);
                self.mode = Mode::InCell;
                self.active.push(Active::Marker);
            }
            Input::End(token) if token.tag == Tag::Tr => {
                if self.in_table_scope(|tag| tag == Tag::Tr) {
                    self.clear_to_table_row_context();
                    self.pop();
                    self.mode = Mode::InTableBody;
                }
            }
            Input::Start(token)
                if matches!(token.tag, Tag::Caption | Tag::Col | Tag::Colgroup | Tag::Tr)
                    || is_table_section(token.tag) =>
            {
                self.leave_row(input);
            }
            Input::End(token) if token.tag == Tag::Table => self.leave_row(input),
            Input::End(token) if is_table_section(token.tag) => {
                if self.in_table_scope(|tag| tag == token.tag) {
                    self.leave_row(input);
                }
            }
            Input::End(token)
                if matches!(
                    token.tag,
                    Tag::Body
                        | Tag::Caption
                        | Tag::Col
                        | Tag::Colgroup
                        | Tag::Html
                        | Tag::Td
                        | Tag::Th
                ) => {}
            _ => self.in_table(input),
        }
    }

    fn leave_row(&mut self, input: Input) {
        if self.in_table_scope(|tag| tag == Tag::Tr) {
            self.clear_to_table_row_context();
            self.pop();
            self.switch(Mode::InTableBody, input);
        }
    }

    fn close_cell(&mut self) -> bool {
        if !self.in_table_scope(|tag| matches!(tag, Tag::Td | Tag::Th)) {
            return false;
        }
        self.generate_implied_end_tags(None);
        self.pop_until(|tag| matches!(tag, Tag::Td | Tag::Th));
        self.clear_active_to_marker();
        self.mode = Mode::InRow;
        true
    }

    fn in_cell(&mut self, input: Input) {
        match input {
            Input::End(token) if matches!(token.tag, Tag::Td | Tag::Th) => {
                if self.in_table_scope(|tag| tag == token.tag) {
                    self.close_cell();
                }
            }
            Input::Start(token)
                if matches!(
                    token.tag,
                    Tag::Caption
                        | Tag::Col
                        | Tag::Colgroup
                        | Tag::Tbody
                        | Tag::Td
                        | Tag::Tfoot
                        | Tag::Th
                        | Tag::Thead
                        | Tag::Tr
                ) =>
            {
                if self.close_cell() {
                    self.dispatch(input);
                }
            }
            Input::End(token)
                if matches!(
                    token.tag,
                    Tag::Body | Tag::Caption | Tag::Col | Tag::Colgroup | Tag::Html
                ) => {}
            Input::End(token)
                if matches!(token.tag, Tag::Table | Tag::Tr) || is_table_section(token.tag) =>
            {
                if self.in_table_scope(|tag| tag == token.tag) && self.close_cell() {
                    self.dispatch(input);
                }
            }
            _ => self.in_body(input),
        }
    }

    fn in_template(&mut self, input: Input) {
        let switch_to = |parser: &mut Self, mode: Mode| {
            parser.template_modes.pop();
            parser.template_modes.push(mode);
            parser.switch(mode, input);
        };
        match input {
            Input::Text(..) | Input::Comment(..) | Input::Doctype(..) => self.in_body(input),
            Input::Start(token) => match token.tag {
                Tag::Base
                | Tag::Basefont
                | Tag::Bgsound
                | Tag::Link
                | Tag::Meta
                | Tag::Noframes
                | Tag::Script
                | Tag::Style
                | Tag::Template
                | Tag::Title => self.in_head(input),
                Tag::Caption | Tag::Colgroup | Tag::Tbody | Tag::Tfoot | Tag::Thead => {
                    switch_to(self, Mode::InTable);
                }
                Tag::Col => switch_to(self, Mode::InColumnGroup),
                Tag::Tr => switch_to(self, Mode::InTableBody),
                Tag::Td | Tag::Th => switch_to(self, Mode::InRow),
                _ => switch_to(self, Mode::InBody),
            },
            Input::End(token) => {
                if token.tag == Tag::Template {
                    self.in_head(input);
                }
            }
            Input::EndOfFile => {
                if self.has_open(Tag::Template) {
                    self.pop_until(|tag| tag == Tag::Template);
                    self.clear_active_to_marker();
                    self.template_modes.pop();
                    self.reset_mode();
                    self.dispatch(input);
                }
            }
        }
    }

    fn after_body(&mut self, input: Input) {
        match input {
            Input::Text(start, end) if self.leading_space(start, end) > 0 => {
                let space = self.leading_space(start, end);
                self.in_body(Input::Text(start, start + space));
                if start + space < end {
                    self.after_body(Input::Text(start + space, end));
                }
            }
            Input::Comment(start, end) => {
                let html = self.open.first().copied().unwrap_or(self.document.root());
                self.append_comment(html, start, end);
            }
            Input::Doctype(..) => {}
            Input::Start(token) if token.tag == Tag::Html => self.in_body(input),
            Input::End(token) if token.tag == Tag::Html => self.mode = Mode::AfterAfterBody,
            Input::EndOfFile => {}
            _ => self.switch(Mode::InBody, input),
        }
    }

    /// Inserts the white space runs of a text and ignores the rest.
    fn insert_space_only(&mut self, start: usize, end: usize) {
        let mut position = start;
        while position < end {
            let space = self.leading_space(position, end);
            if space > 0 {
                self.insert_text(position, position + space);
                position += space;
            }
            let rest = &self.document.source[position..end];
            position += rest.find(is_html_space).unwrap_or(rest.len());
        }
    }

    fn in_frameset(&mut self, input: Input) {
        match input {
            Input::Text(start, end) => self.insert_space_only(start, end),
            Input::Comment(start, end) => self.insert_comment(start, end),
            Input::Start(token) => match token.tag {
                Tag::Html => self.in_body(input),
                Tag::Frameset => {
                    self.insert_html(token);
                }
                Tag::Frame => self.insert_void(token),
                Tag::Noframes => self.in_head(input),
                _ => {}
            },
            Input::End(token) if token.tag == Tag::Frameset && self.current_tag() != Tag::Html => {
                self.pop();
                if self.current_tag() != Tag::Frameset {
                    self.mode = Mode::AfterFrameset;
                }
            }
            _ => {}
        }
    }

    fn after_frameset(&mut self, input: Input) {
        match input {
            Input::Text(start, end) => self.insert_space_only(start, end),
            Input::Comment(start, end) => self.insert_comment(start, end),
            Input::Start(token) if token.tag == Tag::Html => self.in_body(input),
            Input::Start(token) if token.tag == Tag::Noframes => self.in_head(input),
            Input::End(token) if token.tag == Tag::Html => self.mode = Mode::AfterAfterFrameset,
            _ => {}
        }
    }

    fn after_after_body(&mut self, input: Input) {
        match input {
            Input::Comment(start, end) => self.append_comment(self.document.root(), start, end),
            Input::Doctype(..) => {}
            Input::Text(start, end) if self.leading_space(start, end) > 0 => {
                let space = self.leading_space(start, end);
                self.in_body(Input::Text(start, start + space));
                if start + space < end {
                    self.after_after_body(Input::Text(start + space, end));
                }
            }
            Input::Start(token) if token.tag == Tag::Html => self.in_body(input),
            Input::EndOfFile => {}
            _ => self.switch(Mode::InBody, input),
        }
    }

    fn after_after_frameset(&mut self, input: Input) {
        match input {
            Input::Comment(start, end) => self.append_comment(self.document.root(), start, end),
            Input::Text(start, end) => {
                let mut position = start;
                while position < end {
                    let space = self.leading_space(position, end);
                    if space > 0 {
                        self.in_body(Input::Text(position, position + space));
                        position += space;
                    }
                    let rest = &self.document.source[position..end];
                    position += rest.find(is_html_space).unwrap_or(rest.len());
                }
            }
            Input::Start(token) if token.tag == Tag::Html => self.in_body(input),
            Input::Start(token) if token.tag == Tag::Noframes => self.in_head(input),
            _ => {}
        }
    }

    fn in_foreign_content(&mut self, input: Input) {
        match input {
            Input::Text(start, end) => {
                if self.has_content(start, end) {
                    self.frameset_ok = false;
                }
                self.insert_text(start, end);
            }
            Input::Comment(start, end) => self.insert_comment(start, end),
            Input::Doctype(..) | Input::EndOfFile => {}
            Input::Start(token) => {
                if token.tag.breaks_out_of_foreign() || self.is_presentational_font(token) {
                    self.leave_foreign(input);
                } else {
                    let namespace = self.document.namespace(self.current());
                    self.insert_foreign(token, namespace);
                }
            }
            Input::End(token) => {
                if matches!(token.tag, Tag::Br | Tag::P) {
                    self.leave_foreign(input);
                    return;
                }
                for index in (0..self.open.len()).rev() {
                    let id = self.open[index];
                    if self.document.namespace(id) == Namespace::Html {
                        self.by_mode(self.mode, input);
                        return;
                    }
                    let name = &self.document.source[token.name_start..token.name_end];
                    if self.document.name(id).eq_ignore_ascii_case(name) {
                        while self.open.len() > index {
                            self.pop();
                        }
                        return;
                    }
                }
            }
        }
    }

    fn leave_foreign(&mut self, input: Input) {
        while !self.open.is_empty() {
            let id = self.current();
            let tag = self.document.tag(id);
            if self.document.namespace(id) == Namespace::Html
                || tag.is_html_integration_point()
                || tag.is_math_text_integration_point()
            {
                break;
            }
            self.pop();
        }
        self.by_mode(self.mode, input);
    }

    fn is_presentational_font(&self, token: TagToken) -> bool {
        token.tag == Tag::Font
            && attributes(&self.document.source[token.span()]).any(|attribute| {
                ["color", "face", "size"]
                    .iter()
                    .any(|name| attribute.name.eq_ignore_ascii_case(name))
            })
    }

    fn is_hidden_input(&self, token: TagToken) -> bool {
        attributes(&self.document.source[token.span()])
            .find(|attribute| attribute.name.eq_ignore_ascii_case("type"))
            .and_then(|attribute| attribute.value)
            .is_some_and(|value| value.eq_ignore_ascii_case("hidden"))
    }

    fn reset_mode(&mut self) {
        let mut mode = Mode::InBody;
        for index in (0..self.open.len()).rev() {
            let last = index == 0;
            mode = match self.document.tag(self.open[index]) {
                Tag::Td | Tag::Th if !last => Mode::InCell,
                Tag::Tr => Mode::InRow,
                Tag::Tbody | Tag::Thead | Tag::Tfoot => Mode::InTableBody,
                Tag::Caption => Mode::InCaption,
                Tag::Colgroup => Mode::InColumnGroup,
                Tag::Table => Mode::InTable,
                Tag::Template => *self.template_modes.last().unwrap_or(&Mode::InBody),
                Tag::Head if !last => Mode::InHead,
                Tag::Body => Mode::InBody,
                Tag::Frameset => Mode::InFrameset,
                Tag::Html if self.head.is_none() => Mode::BeforeHead,
                Tag::Html => Mode::AfterHead,
                _ if last => Mode::InBody,
                _ => continue,
            };
            break;
        }
        self.mode = mode;
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
        if !self.has_clone_budget() {
            self.any_other_end_tag(token);
            return;
        }
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
            if !self.is_in_scope(formatting) {
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
                let clone = self.clone_element(node);
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
            let (parent, before) = self.insertion_place(common_ancestor);
            self.place_node(last, parent, before);
            let clone = self.clone_element(formatting);
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
            if let Some(position) = self.open.iter().position(|&id| id == formatting) {
                self.document.set_open(formatting, false);
                self.open.remove(position);
            }
            let position = self.open.iter().position(|&id| id == furthest);
            let position = position.map_or(self.open.len(), |position| position + 1);
            self.document.set_open(clone, true);
            self.open.insert(position, clone);
        }
    }

    fn is_in_scope(&self, element: NodeId) -> bool {
        for &id in self.open.iter().rev() {
            if id == element {
                return true;
            }
            if is_scope_boundary(self.document.tag(id)) {
                return false;
            }
        }
        false
    }

    fn insert_formatting(&mut self, token: TagToken) {
        self.reconstruct_active_formatting();
        let element = self.insert_html(token);
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
            if !self.has_clone_budget() {
                // The remaining entries are all closed: their formatting ends here.
                self.active.truncate(index);
                return;
            }
            if self.open.len() >= MAX_DEPTH {
                self.pop();
            }
            let clone = self.clone_element(element);
            self.insert_node(clone);
            self.push_open(clone);
            self.active[index] = Active::Element(clone);
        }
    }

    fn has_clone_budget(&mut self) -> bool {
        let allowed = FREE_CLONES + self.processed / INPUT_BYTES_PER_CLONE;
        self.formatting_limited |= self.clones >= allowed;
        self.clones < allowed
    }

    fn clone_element(&mut self, id: NodeId) -> NodeId {
        self.clones += 1;
        self.document.clone_element(id)
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

    fn has_open(&self, tag: Tag) -> bool {
        self.open.iter().any(|&id| self.document.tag(id) == tag)
    }

    fn in_table_scope(&self, target: impl Fn(Tag) -> bool) -> bool {
        self.in_scope(target, |tag| {
            matches!(tag, Tag::Html | Tag::Table | Tag::Template)
        })
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

    fn generate_implied_end_tags(&mut self, except: Option<Tag>) {
        loop {
            let tag = self.current_tag();
            if !tag.has_implied_end() || Some(tag) == except {
                return;
            }
            self.pop();
        }
    }

    fn generate_implied_end_tags_thoroughly(&mut self) {
        loop {
            let tag = self.current_tag();
            if !tag.has_implied_end()
                && !matches!(
                    tag,
                    Tag::Caption
                        | Tag::Colgroup
                        | Tag::Tbody
                        | Tag::Td
                        | Tag::Tfoot
                        | Tag::Th
                        | Tag::Thead
                        | Tag::Tr
                )
            {
                return;
            }
            self.pop();
        }
    }

    fn token_name(&self, token: TagToken) -> &str {
        &self.document.source[token.name_start..token.name_end]
    }

    fn same_name(&self, id: NodeId, token: TagToken) -> bool {
        let tag = self.document.tag(id);
        tag == token.tag
            && self.document.namespace(id) == Namespace::Html
            && (tag != Tag::Other
                || self
                    .document
                    .name(id)
                    .eq_ignore_ascii_case(self.token_name(token)))
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

    fn template_contents(&self, template: NodeId) -> NodeId {
        self.document.first_child(template).unwrap_or(template)
    }

    /// Where new content goes when `target` is the node to hold it: in front of the table
    /// for misplaced table content, and inside the contents of a template.
    fn insertion_place(&self, target: NodeId) -> (NodeId, Option<NodeId>) {
        let tag = self.document.tag(target);
        if self.foster
            && matches!(
                tag,
                Tag::Table | Tag::Tbody | Tag::Tfoot | Tag::Thead | Tag::Tr
            )
        {
            let position = |wanted: Tag| {
                self.open
                    .iter()
                    .rposition(|&id| self.document.tag(id) == wanted)
            };
            let table = position(Tag::Table);
            if let Some(template) = position(Tag::Template)
                && table.is_none_or(|table| template > table)
            {
                return (self.template_contents(self.open[template]), None);
            }
            let Some(table_index) = table else {
                return (self.open[0], None);
            };
            let table = self.open[table_index];
            return match self.document.parent(table) {
                Some(parent) => (parent, Some(table)),
                None => (self.open[table_index.saturating_sub(1)], None),
            };
        }
        if tag == Tag::Template && self.document.namespace(target) == Namespace::Html {
            return (self.template_contents(target), None);
        }
        (target, None)
    }

    fn place_node(&mut self, node: NodeId, parent: NodeId, before: Option<NodeId>) {
        match before {
            Some(before) => self.document.insert_before(parent, node, before),
            None => self.document.append(parent, node),
        }
    }

    fn insert_node(&mut self, node: NodeId) {
        let (parent, before) = self.insertion_place(self.current());
        self.place_node(node, parent, before);
    }

    fn make_room(&mut self) {
        if self.open.len() >= MAX_DEPTH {
            self.pop();
        }
    }

    fn insert_html(&mut self, token: TagToken) -> NodeId {
        self.insert_open(token.tag, token.span())
    }

    /// Creates an element the source left out; it has an empty span and no attributes.
    fn insert_implied(&mut self, tag: Tag) -> NodeId {
        let end = self.processed;
        self.insert_open(tag, end..end)
    }

    fn insert_open(&mut self, tag: Tag, span: Range<usize>) -> NodeId {
        self.make_room();
        let element = self.document.create(NodeKind::Element, tag, span);
        self.insert_node(element);
        self.push_open(element);
        if tag == Tag::Template {
            let contents = self.document.create(NodeKind::Fragment, Tag::Other, 0..0);
            self.document.append(element, contents);
        }
        element
    }

    fn insert_foreign(&mut self, token: TagToken, namespace: Namespace) {
        self.make_room();
        let tag = self.foreign_tag(token, namespace);
        let element = self.document.create(NodeKind::Element, tag, token.span());
        self.document.set_namespace(element, namespace);
        self.insert_node(element);
        if !token.self_closing {
            self.push_open(element);
        }
    }

    fn foreign_tag(&self, token: TagToken, namespace: Namespace) -> Tag {
        let name = self.token_name(token).to_ascii_lowercase();
        match (namespace, name.as_str()) {
            (Namespace::Svg, "svg") if token.tag == Tag::Svg => Tag::Svg,
            (Namespace::MathMl, "math") if token.tag == Tag::Math => Tag::Math,
            (Namespace::Svg, "foreignobject") => Tag::SvgForeignObject,
            (Namespace::Svg, "desc") => Tag::SvgDesc,
            (Namespace::Svg, "title") => Tag::SvgTitle,
            (Namespace::MathMl, "mi") => Tag::MathMi,
            (Namespace::MathMl, "mo") => Tag::MathMo,
            (Namespace::MathMl, "mn") => Tag::MathMn,
            (Namespace::MathMl, "ms") => Tag::MathMs,
            (Namespace::MathMl, "mtext") => Tag::MathMtext,
            (Namespace::MathMl, "annotation-xml") => {
                let encoding = attributes(&self.document.source[token.span()])
                    .find(|attribute| attribute.name.eq_ignore_ascii_case("encoding"))
                    .and_then(|attribute| attribute.value);
                if encoding.is_some_and(|value| {
                    value.eq_ignore_ascii_case("text/html")
                        || value.eq_ignore_ascii_case("application/xhtml+xml")
                }) {
                    Tag::MathAnnotationHtml
                } else {
                    Tag::MathAnnotationXml
                }
            }
            _ => Tag::Other,
        }
    }

    fn insert_void(&mut self, token: TagToken) {
        let tag = if token.tag == Tag::Image {
            Tag::Img
        } else {
            token.tag
        };
        let element = self.document.create(NodeKind::Element, tag, token.span());
        self.insert_node(element);
    }

    /// Inserts an element whose content the tokenizer reads as raw text.
    fn insert_raw(&mut self, token: TagToken) {
        self.insert_html(token);
        self.scanner
            .enter_raw_text(token.name_start..token.name_end);
        self.original_mode = self.mode;
        self.mode = Mode::Text;
    }

    fn insert_text(&mut self, start: usize, end: usize) {
        let span = start..end;
        let (parent, before) = self.insertion_place(self.current());
        let previous = match before {
            Some(before) => self.document.previous_sibling(before),
            None => self.document.last_child(parent),
        };
        if let Some(previous) = previous
            && self.document.extend_text(previous, &span)
        {
            return;
        }
        let text = self.document.create(NodeKind::Text, Tag::Other, span);
        self.place_node(text, parent, before);
    }

    fn insert_comment(&mut self, start: usize, end: usize) {
        let comment = self
            .document
            .create(NodeKind::Comment, Tag::Other, start..end);
        self.insert_node(comment);
    }

    fn append_comment(&mut self, parent: NodeId, start: usize, end: usize) {
        let comment = self
            .document
            .create(NodeKind::Comment, Tag::Other, start..end);
        self.document.append(parent, comment);
    }

    fn merge_attributes(&mut self, element: NodeId, token: TagToken) {
        if attributes(&self.document.source[token.span()])
            .next()
            .is_some()
        {
            self.document.merge_attributes(element, &token.span());
        }
    }

    fn leading_space(&self, start: usize, end: usize) -> usize {
        let text = &self.document.source[start..end];
        text.len() - text.trim_start_matches(is_html_space).len()
    }

    /// Whether a text holds anything besides white space and the NUL characters dropped later.
    fn has_content(&self, start: usize, end: usize) -> bool {
        self.document.source[start..end]
            .chars()
            .any(|c| !is_html_space(c) && c != '\0')
    }
}

fn is_scope_boundary(tag: Tag) -> bool {
    tag.is_foreign_boundary()
        || matches!(
            tag,
            Tag::Applet
                | Tag::Caption
                | Tag::Html
                | Tag::Marquee
                | Tag::Object
                | Tag::Select
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
