# ron
Memory-safe web engine library in Rust. It takes UTF-8 text, never opens files or
sockets itself, and has no GUI, so a browser and a crawler can share it. The browser
built on it is a separate project, `ron-browser`.

The current stage is HTML parsing into a document tree; there is no CSS or rendering yet.

## HTML parser

`ron::html::parse` builds a `Document` from a complete page. `ron::html::Parser` does the
same for a page that arrives in pieces: call `feed` with each piece and `finish` at the
end. After every `feed`, `document()` returns a valid tree of what is known so far, so a
page can be shown before it has fully arrived.

The tree follows the HTML tree construction rules, including implied elements, implied
end tags and the repair of misnested formatting. Not implemented yet: moving misplaced
table content in front of the table, `<template>` contents, and the separate rules of SVG
and MathML. Character references are not decoded yet.

Nodes take 28 bytes each and refer to the source, which the document keeps; text and
attributes are not copied. As a result text interrupted by an ignored tag stays as two
adjacent text nodes, and names of unknown elements keep the case they were written in.

At most 512 elements stay open, with further ones becoming siblings. Repairing misnested
formatting can leave the tree deeper than that, so code walking it must not recurse. A
document stops growing at about 16 million nodes or 4 GiB of source; `is_truncated` reports
that.

## HTML tokenizer

`ron::html::Tokenizer` splits a document into text, tags, comments, declarations and
raw-text content (`<script>`, `<style>`, `<title>`...). It accepts every input, including
broken markup, and runs in linear time. Attributes are read on demand from the source
text of a tag with `ron::html::attributes`, without copying. Character references are
not decoded yet.

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
cargo test
RON_CORPUS_DIR=pages cargo test --release -- --ignored  # also real .html files from a directory
```

`generated/html_parser.rs` is generated and checked in, so building needs only Cargo.
After changing the grammar, regenerate it with `ag` from
[Borsuk](https://github.com/parstools/borsuk):

```sh
ag --emit-rust-parser generated/html_parser.rs grammars/html.ag
```
