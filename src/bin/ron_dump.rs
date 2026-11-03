//! Prints the tokens of a UTF-8 HTML file, or their statistics with `--stats`.
//! With `--chunk N` the file goes through the stream tokenizer in pieces of N bytes.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

use ron::html::{StreamTokenizer, Token, TokenKind, Tokenizer, attributes, tag_name};

const USAGE: &str = "usage: ron-dump [--stats] [--chunk BYTES] FILE";

fn for_each_token(source: &str, chunk: Option<usize>, mut visit: impl FnMut(Token)) {
    let Some(size) = chunk else {
        Tokenizer::new(source).for_each(visit);
        return;
    };
    let mut stream = StreamTokenizer::new();
    let mut rest = source;
    while !rest.is_empty() {
        let mut end = size.min(rest.len());
        while !rest.is_char_boundary(end) {
            end += 1;
        }
        let (piece, tail) = rest.split_at(end);
        stream.feed(piece);
        rest = tail;
        while let Some(token) = stream.next_token() {
            visit(token);
        }
    }
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

fn main() -> ExitCode {
    let mut stats = false;
    let mut chunk = None;
    let mut path = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--stats" => stats = true,
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
    if stats {
        print_stats(&source, chunk);
    } else {
        print_tokens(&source, chunk);
    }
    ExitCode::SUCCESS
}
