//! Prints the tokens of a UTF-8 HTML file, or their statistics with `--stats`.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

use ron::html::{TokenKind, Tokenizer, attributes, tag_name};

fn print_tokens(source: &str) {
    for token in Tokenizer::new(source) {
        let text = &source[token.span.clone()];
        println!("{:?} {:?} {:?}", token.kind, token.span, text);
        if matches!(token.kind, TokenKind::StartTag | TokenKind::EndTag) {
            println!("  name {:?}", tag_name(text));
            for attribute in attributes(text) {
                println!("  attribute {:?} = {:?}", attribute.name, attribute.value);
            }
        }
    }
}

fn print_stats(source: &str) {
    let started = Instant::now();
    let mut counts = BTreeMap::new();
    for token in Tokenizer::new(source) {
        *counts.entry(format!("{:?}", token.kind)).or_insert(0_usize) += 1;
    }
    let elapsed = started.elapsed();
    println!("bytes {}", source.len());
    println!("time {elapsed:?}");
    for (kind, count) in &counts {
        println!("{kind} {count}");
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (stats, path) = match arguments.as_slice() {
        [path] => (false, path),
        [flag, path] if flag == "--stats" => (true, path),
        _ => {
            eprintln!("usage: ron-dump [--stats] FILE");
            return ExitCode::from(2);
        }
    };
    let source = match std::fs::read(path).map(String::from_utf8) {
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
        print_stats(&source);
    } else {
        print_tokens(&source);
    }
    ExitCode::SUCCESS
}
