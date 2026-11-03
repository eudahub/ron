//! Reads a doctype from its source text and decides the rendering mode it asks for.

/// The parts of a `<!DOCTYPE ...>` declaration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Doctype {
    /// Lowercase name; empty when missing.
    pub name: String,
    pub public_id: Option<String>,
    pub system_id: Option<String>,
    /// The declaration is malformed in a way that asks for quirks mode.
    pub force_quirks: bool,
}

fn skip_space(text: &str) -> &str {
    text.trim_start_matches([' ', '\t', '\r', '\n', '\x0c'])
}

fn clean(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\0', "\u{fffd}")
}

/// Reads a quoted value. The second result is what follows it, when the quote was closed.
fn quoted(text: &str) -> Option<(String, Option<&str>)> {
    let quote = text.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let body = &text[1..];
    Some(match body.find(quote) {
        Some(end) => (clean(&body[..end]), Some(&body[end + 1..])),
        None => (clean(body), None),
    })
}

impl Doctype {
    /// Parses the source text of a doctype, from `<!` up to and including any `>`.
    pub(super) fn parse(raw: &str) -> Self {
        let body = raw.strip_suffix('>').unwrap_or(raw);
        let rest = skip_space(body.get(9..).unwrap_or(""));
        let name_end = rest
            .find([' ', '\t', '\r', '\n', '\x0c'])
            .unwrap_or(rest.len());
        let (name, rest) = rest.split_at(name_end);
        let mut doctype = Self {
            name: clean(name).to_ascii_lowercase(),
            force_quirks: name.is_empty(),
            ..Self::default()
        };
        let rest = skip_space(rest);
        if rest.is_empty() {
            return doctype;
        }
        let keyword = rest.get(..6).unwrap_or("");
        if keyword.eq_ignore_ascii_case("public") {
            doctype.read_public(&rest[6..]);
        } else if keyword.eq_ignore_ascii_case("system") {
            doctype.read_system(&rest[6..]);
        } else {
            doctype.force_quirks = true;
        }
        doctype
    }

    fn read_public(&mut self, text: &str) {
        let Some((public_id, after)) = quoted(skip_space(text)) else {
            self.force_quirks = true;
            return;
        };
        self.public_id = Some(public_id);
        let Some(after) = after else {
            self.force_quirks = true;
            return;
        };
        let after = skip_space(after);
        if after.is_empty() {
            return;
        }
        if let Some((system_id, end)) = quoted(after) {
            self.system_id = Some(system_id);
            self.force_quirks |= end.is_none();
        } else {
            self.force_quirks = true;
        }
    }

    fn read_system(&mut self, text: &str) {
        let Some((system_id, end)) = quoted(skip_space(text)) else {
            self.force_quirks = true;
            return;
        };
        self.system_id = Some(system_id);
        self.force_quirks |= end.is_none();
    }

    /// Whether this doctype selects quirks mode, by the lists of the HTML standard.
    pub(super) fn is_quirks(&self) -> bool {
        if self.force_quirks || self.name != "html" {
            return true;
        }
        let public = self.public_id.as_deref().unwrap_or("").to_ascii_lowercase();
        let system = self.system_id.as_deref().unwrap_or("").to_ascii_lowercase();
        let html_4 = public.starts_with("-//w3c//dtd html 4.01 frameset//")
            || public.starts_with("-//w3c//dtd html 4.01 transitional//");
        QUIRKS_PUBLIC_IDS.contains(&public.as_str())
            || system == "http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd"
            || QUIRKS_PUBLIC_PREFIXES
                .iter()
                .any(|prefix| public.starts_with(prefix))
            || (self.system_id.is_none() && html_4)
    }
}

const QUIRKS_PUBLIC_IDS: [&str; 3] = [
    "-//w3o//dtd w3 html strict 3.0//en//",
    "-/w3c/dtd html 4.0 transitional/en",
    "html",
];

const QUIRKS_PUBLIC_PREFIXES: [&str; 55] = [
    "+//silmaril//dtd html pro v0r11 19970101//",
    "-//as//dtd html 3.0 aswedit + extensions//",
    "-//advasoft ltd//dtd html 3.0 aswedit + extensions//",
    "-//ietf//dtd html 2.0 level 1//",
    "-//ietf//dtd html 2.0 level 2//",
    "-//ietf//dtd html 2.0 strict level 1//",
    "-//ietf//dtd html 2.0 strict level 2//",
    "-//ietf//dtd html 2.0 strict//",
    "-//ietf//dtd html 2.0//",
    "-//ietf//dtd html 2.1e//",
    "-//ietf//dtd html 3.0//",
    "-//ietf//dtd html 3.2 final//",
    "-//ietf//dtd html 3.2//",
    "-//ietf//dtd html 3//",
    "-//ietf//dtd html level 0//",
    "-//ietf//dtd html level 1//",
    "-//ietf//dtd html level 2//",
    "-//ietf//dtd html level 3//",
    "-//ietf//dtd html strict level 0//",
    "-//ietf//dtd html strict level 1//",
    "-//ietf//dtd html strict level 2//",
    "-//ietf//dtd html strict level 3//",
    "-//ietf//dtd html strict//",
    "-//ietf//dtd html//",
    "-//metrius//dtd metrius presentational//",
    "-//microsoft//dtd internet explorer 2.0 html strict//",
    "-//microsoft//dtd internet explorer 2.0 html//",
    "-//microsoft//dtd internet explorer 2.0 tables//",
    "-//microsoft//dtd internet explorer 3.0 html strict//",
    "-//microsoft//dtd internet explorer 3.0 html//",
    "-//microsoft//dtd internet explorer 3.0 tables//",
    "-//netscape comm. corp.//dtd html//",
    "-//netscape comm. corp.//dtd strict html//",
    "-//o'reilly and associates//dtd html 2.0//",
    "-//o'reilly and associates//dtd html extended 1.0//",
    "-//o'reilly and associates//dtd html extended relaxed 1.0//",
    "-//sq//dtd html 2.0 hotmetal + extensions//",
    "-//softquad software//dtd hotmetal pro 6.0::19990601::extensions to html 4.0//",
    "-//softquad//dtd hotmetal pro 4.0::19971010::extensions to html 4.0//",
    "-//spyglass//dtd html 2.0 extended//",
    "-//sun microsystems corp.//dtd hotjava html//",
    "-//sun microsystems corp.//dtd hotjava strict html//",
    "-//w3c//dtd html 3 1995-03-24//",
    "-//w3c//dtd html 3.2 draft//",
    "-//w3c//dtd html 3.2 final//",
    "-//w3c//dtd html 3.2//",
    "-//w3c//dtd html 3.2s draft//",
    "-//w3c//dtd html 4.0 frameset//",
    "-//w3c//dtd html 4.0 transitional//",
    "-//w3c//dtd html experimental 19960712//",
    "-//w3c//dtd html experimental 970421//",
    "-//w3c//dtd w3 html//",
    "-//w3o//dtd w3 html 3.0//",
    "-//webtechs//dtd mozilla html 2.0//",
    "-//webtechs//dtd mozilla html//",
];
