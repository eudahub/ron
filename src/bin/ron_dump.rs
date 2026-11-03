//! Inspects how a UTF-8 HTML file is parsed: its tokens, token statistics with `--stats`,
//! the tree with `--tree`, the memory used to build it with `--memory`, or how the CSS
//! of a stylesheet or of a page's `<style>` elements parses with `--css`.
//! With `--chunk N` the file goes through the stream tokenizer or parser in pieces of N bytes.

use std::collections::BTreeMap;
use std::io::Read;
use std::process::ExitCode;
use std::time::Instant;

use ron::css::Stylesheet;
use ron::html::{
    Document, NodeKind, Parser, StreamTokenizer, Tag, Token, TokenKind, Tokenizer, attributes,
    parse, tag_name,
};

const USAGE: &str = "usage: ron-dump [--stats | --tree | --memory | --css] [--chunk BYTES] FILE";

/// Calls `visit` with consecutive pieces of at most `size` bytes, cut at character boundaries.
fn for_each_piece(source: &str, size: usize, mut visit: impl FnMut(&str)) {
    let mut rest = source;
    while !rest.is_empty() {
        let mut end = size.min(rest.len());
        while !rest.is_char_boundary(end) {
            end += 1;
        }
        let (piece, tail) = rest.split_at(end);
        visit(piece);
        rest = tail;
    }
}

fn for_each_token(source: &str, chunk: Option<usize>, mut visit: impl FnMut(Token)) {
    let Some(size) = chunk else {
        Tokenizer::new(source).for_each(visit);
        return;
    };
    let mut stream = StreamTokenizer::new();
    for_each_piece(source, size, |piece| {
        stream.feed(piece);
        while let Some(token) = stream.next_token() {
            visit(token);
        }
    });
    stream.finish();
    while let Some(token) = stream.next_token() {
        visit(token);
    }
}

fn print_tokens(source: &str, chunk: Option<usize>) {
    for_each_token(source, chunk, |token| {
        let text = &source[token.span.clone()];
        println!("{:?} {:?} {:?}", token.kind, token.span, text);
        if matches!(token.kind, TokenKind::StartTag | TokenKind::EndTag) {
            println!("  name {:?}", tag_name(text));
            for attribute in attributes(text) {
                println!("  attribute {:?} = {:?}", attribute.name, attribute.value);
            }
        }
    });
}

fn print_stats(source: &str, chunk: Option<usize>) {
    let started = Instant::now();
    let mut counts = BTreeMap::new();
    for_each_token(source, chunk, |token| {
        *counts.entry(format!("{:?}", token.kind)).or_insert(0_usize) += 1;
    });
    let elapsed = started.elapsed();
    println!("bytes {}", source.len());
    println!("time {elapsed:?}");
    for (kind, count) in &counts {
        println!("{kind} {count}");
    }
}

fn print_tree(document: &Document) {
    let root = document.root();
    for id in document.descendants(root).skip(1) {
        let mut depth = 0;
        let mut ancestor = document.parent(id);
        while let Some(parent) = ancestor.filter(|&parent| parent != root) {
            depth += 1;
            ancestor = document.parent(parent);
        }
        let indent = "  ".repeat(depth);
        match document.kind(id) {
            NodeKind::Element => {
                println!("{indent}<{}>", document.name(id));
                for attribute in document.attributes(id) {
                    println!("{indent}  {} = {:?}", attribute.name, attribute.value);
                }
            }
            NodeKind::Document | NodeKind::Fragment => {}
            _ => println!("{indent}{:?}", document.text(id)),
        }
    }
}

fn print_parsed_tree(source: &str, chunk: Option<usize>) {
    let mut parser = Parser::new();
    for_each_piece(source, chunk.unwrap_or(source.len().max(1)), |piece| {
        parser.feed(piece);
    });
    parser.finish();
    print_tree(parser.document());
}

/// Reads a value such as `VmHWM` from the kernel's process status, in kilobytes.
fn process_status_kb(field: &str) -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with(field))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Parses the file as it is read, as with a download, and reports the memory it took.
fn print_memory(path: &str, chunk: Option<usize>) -> Result<(), String> {
    let describe = |error: std::io::Error| format!("{path}: {error}");
    let mut file = std::fs::File::open(path).map_err(describe)?;
    let start_kb = process_status_kb("VmRSS").unwrap_or(0);
    let started = Instant::now();
    let mut parser = Parser::new();
    let mut buffer = vec![0_u8; chunk.unwrap_or(65536)];
    // The bytes of a character cut by the end of the previous read.
    let mut kept = 0;
    loop {
        let read = file.read(&mut buffer[kept..]).map_err(describe)?;
        if read == 0 {
            break;
        }
        let filled = kept + read;
        let valid = match std::str::from_utf8(&buffer[..filled]) {
            Ok(piece) => piece,
            Err(error) if error.error_len().is_none() => {
                std::str::from_utf8(&buffer[..error.valid_up_to()]).unwrap_or("")
            }
            Err(error) => return Err(format!("{path}: not UTF-8: {error}")),
        };
        parser.feed(valid);
        let used = valid.len();
        buffer.copy_within(used..filled, 0);
        kept = filled - used;
        if buffer.len() < kept + 4 {
            buffer.resize(kept + 4, 0);
        }
    }
    if kept > 0 {
        return Err(format!("{path}: not UTF-8: cut character at the end"));
    }
    parser.finish();
    let elapsed = started.elapsed();
    let peak_kb = process_status_kb("VmHWM").unwrap_or(0);
    let truncated = parser.is_truncated();
    let document = parser.document();
    let source = document.source_capacity_bytes();
    let nodes = document.nodes_capacity_bytes();
    #[allow(clippy::cast_precision_loss)]
    let ratio = (source + nodes) as f64 / source.max(1) as f64;
    println!("source_bytes {source}");
    println!("nodes {}", document.node_count());
    println!("node_bytes {nodes}");
    println!("document_to_source_ratio {ratio:.2}");
    println!("process_kb_at_start {start_kb}");
    println!("process_kb_peak {peak_kb}");
    println!("time {elapsed:?}");
    if truncated {
        println!("truncated");
    }
    Ok(())
}

fn print_css(source: &str, path: &str) {
    let sheets: Vec<String> = if path.ends_with(".css") {
        vec![source.to_owned()]
    } else {
        let document = parse(source);
        document
            .descendants(document.root())
            .filter(|&id| document.tag(id) == Tag::Style)
            .filter_map(|id| document.first_child(id))
            .map(|text| document.decoded_text(text).into_owned())
            .collect()
    };
    let started = Instant::now();
    let parsed: Vec<Stylesheet> = sheets
        .iter()
        .map(|sheet| Stylesheet::parse(sheet))
        .collect();
    let elapsed = started.elapsed();
    let count = |measure: fn(&Stylesheet) -> usize| parsed.iter().map(measure).sum::<usize>();
    println!("stylesheets {}", parsed.len());
    println!("bytes {}", sheets.iter().map(String::len).sum::<usize>());
    println!("rules {}", count(|sheet| sheet.rules.len()));
    let selectors = count(|sheet| sheet.rules.iter().map(|rule| rule.selectors.len()).sum());
    println!("selectors {selectors}");
    let declarations = count(|sheet| sheet.rules.iter().map(|rule| rule.declarations.len()).sum());
    println!("declarations {declarations}");
    println!("time {elapsed:?}");
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Output {
    Tokens,
    Stats,
    Tree,
    Memory,
    Css,
}

fn main() -> ExitCode {
    let mut output = Output::Tokens;
    let mut chunk = None;
    let mut path = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--stats" => output = Output::Stats,
            "--tree" => output = Output::Tree,
            "--memory" => output = Output::Memory,
            "--css" => output = Output::Css,
            "--chunk" => {
                chunk = arguments.next().and_then(|size| size.parse().ok());
                if chunk.is_none_or(|size: usize| size == 0) {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            }
            _ if path.is_none() => path = Some(argument),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(path) = path else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if output == Output::Memory {
        return match print_memory(&path, chunk) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
        };
    }
    let source = match std::fs::read(&path).map(String::from_utf8) {
        Ok(Ok(source)) => source,
        Ok(Err(error)) => {
            eprintln!("{path}: not UTF-8: {error}");
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    match output {
        Output::Tokens => print_tokens(&source, chunk),
        Output::Stats => print_stats(&source, chunk),
        Output::Tree => print_parsed_tree(&source, chunk),
        Output::Css => print_css(&source, &path),
        Output::Memory => {}
    }
    ExitCode::SUCCESS
}
