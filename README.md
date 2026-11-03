# ron
Memory-safe web engine library in Rust. It takes UTF-8 text, never opens files or
sockets itself, and has no GUI, so a browser and a crawler can share it. The browser
built on it is a separate project, `ron-browser`.

The current stage is parsing HTML and CSS and computing the style of every element. Layout
and rendering belong to the browser, which is a separate project.

## HTML parser

`ron::html::parse` builds a `Document` from a complete page. `ron::html::Parser` does the
same for a page that arrives in pieces: call `feed` with each piece and `finish` at the
end. After every `feed`, `document()` returns a valid tree of what is known so far, so a
page can be shown before it has fully arrived.

The tree follows the insertion modes of the HTML standard, including misplaced table
content, `<template>` contents, frameset documents, SVG and MathML. Of the 1592 html5lib tree
construction cases that apply, 1587 pass. The rest are `<selectedcontent>`, which copies the
selected option, and a newline after `<pre>` written as a character reference. Scripting
counts as disabled and scripts do not run. Character references stay as written in the
source; `decoded_text` and `decoded_value` decode them when read. Attribute names in SVG and
MathML keep the form they were written in.

Nodes take 28 bytes each and refer to the source, which the document keeps; text and
attributes are not copied. As a result text interrupted by an ignored tag stays as two
adjacent text nodes; `name` keeps the case of unknown names and `local_name` lowercases them.

At most 512 elements stay open, with further ones becoming siblings. Repairing misnested
formatting can leave the tree deeper than that, so code walking it must not recurse. A
document stops growing at about 16 million nodes or 4 GiB of source; `is_truncated` reports
that. Reopening and repairing formatting clones elements, up to 1000 clones plus one per 4
bytes of input. Beyond that text is still shown but loses the formatting that would have
needed a clone, and `is_formatting_limited` reports it.

## CSS parser

`ron::css::Stylesheet::parse` reads a stylesheet into rules with their selectors and
declarations. It accepts every input: as CSS Syntax requires, an error drops one rule or
declaration, and the input ending inside a block closes it. Values stay as written in the
source. `@media` conditions and `@import` targets are kept but not evaluated, and `@supports`
content is read as if its condition held. Nested style rules are skipped.

`grammars/css.ag` holds the tokens and an LALR(1) grammar for selector lists, which decides
validity, since one invalid selector voids its whole rule. `ron::css::matches` tests an
element against a selector. It handles type, class, id and attribute selectors, all four
combinators and the structural pseudo-classes, `:not()`, `:is()` and `:where()`. States of a
live page, such as `:hover`, are valid but do not match yet, and `:has()` counts as invalid.

## Computed styles

`ron::css::compute_styles` gives every element its computed style: the default styles of the
HTML standard, then old presentational attributes such as `bgcolor` and `<font size>`, then
author rules and `style` attributes, with specificity, `!important`, inheritance and `@media`
conditions. `document_stylesheets` collects a page's `<style>` elements; the caller fetches
any others. Elements with equal styles share one. Generic `monospace` text inherits a smaller
size, 12 pixels where others have 16, as in Firefox on Linux.

The properties cover display, floats, the box model, borders, backgrounds, fonts, text and
lists. A value ron cannot read drops its declaration. That includes `var()` and `calc()`, on
which modern pages rely, so such pages lose part of their styling for now. The styles of
`::before` and `::after` are not computed. Selector matching has a step budget, so patterns
such as `.a ~ b` on huge lists stop matching instead of taking hours; `is_limited` reports it.

## HTML tokenizer

`ron::html::Tokenizer` splits a document into text, tags, comments, declarations and
raw-text content (`<script>`, `<style>`, `<title>`...). It accepts every input, including
broken markup, and runs in linear time. Attributes are read on demand from the source
text of a tag with `ron::html::attributes`, without copying.

`ron::html::StreamTokenizer` does the same for a document that arrives in pieces, as from
a network. Call `feed` with each piece, `next_token` until it returns `None`, and `finish`
at the end. A token is returned once later input cannot change it, so a page can be shown
before it has fully arrived; text and raw text come as far as they have arrived, possibly
as several adjacent tokens. Pieces must be valid UTF-8, so a decoder has to keep a
multi-byte character cut by a piece boundary for the next piece.

Real pages do not nest properly, so `grammars/html.ag` describes a flat stream of tokens
and nesting is left to a tree builder. Input must already be UTF-8; detecting and
converting legacy encodings is up to the calling program.

## Building

```sh
cargo build --release
target/release/ron-dump page.html          # print tokens and attributes
target/release/ron-dump --stats page.html  # token counts and tokenizing time
target/release/ron-dump --chunk 1400 page.html  # the same through the stream tokenizer
target/release/ron-dump --tree page.html   # the document tree
target/release/ron-dump --memory page.html # memory and time while parsing as it is read
target/release/ron-dump --css page.html    # how a stylesheet or a page's <style> parses
target/release/ron-dump --style page.html  # time and memory of the computed styles
target/release/ron-dump --computed page.html  # the style of each element
cargo test
RON_CORPUS_DIR=pages cargo test --release -- --ignored  # also real .html files from a directory
RON_HTML5LIB_DIR=dat cargo test --release --test html5lib -- --ignored --nocapture
```

`generated/html_parser.rs` and `generated/css_parser.rs` are generated and checked in, so
building needs only Cargo. After changing a grammar, regenerate them with `ag` from
[Borsuk](https://github.com/parstools/borsuk):

```sh
ag --emit-rust-parser generated/html_parser.rs grammars/html.ag
ag --emit-rust-parser generated/css_parser.rs grammars/css.ag
```

`RON_HTML5LIB_DIR` names a directory of html5lib tree construction `.dat` files. They moved
to web-platform-tests in 2026; the last version in that format is in the history of
[html5lib-tests](https://github.com/html5lib/html5lib-tests), in the parent of commit
`224991e`. `generated/html_entities.rs` comes from the `entities.json` of the HTML standard
through `tools/generate_entities.py`.
