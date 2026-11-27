//! Whole spoken phrases become line breaks, quotation marks and parentheses: "new line",
//! "novo parágrafo", "abre aspas" … "fecha aspas", "open parenthesis" … "close parenthesis",
//! and in English "quote" … "unquote" around words. Matching ignores case and accents and needs
//! word boundaries, so "newline" or "nova linhagem" stay as they are. The commas, periods and
//! spaces a model puts around a command go with it.

use super::{Language, fold, phrase_at, skip, trim_end, word_start};

#[derive(Clone, Copy)]
enum Command {
    Break(&'static str),
    Open(char),
    Close(char),
}

const PARAGRAPH: Command = Command::Break("\n\n");
const LINE: Command = Command::Break("\n");

const ENGLISH: &[(&[&str], Command)] = &[
    (&["new", "paragraph"], PARAGRAPH),
    (&["new", "line"], LINE),
    (&["line", "break"], LINE),
    (&["open", "quote"], Command::Open('"')),
    (&["close", "quote"], Command::Close('"')),
    (&["open", "parenthesis"], Command::Open('(')),
    (&["close", "parenthesis"], Command::Close(')')),
    (&["open", "paren"], Command::Open('(')),
    (&["close", "paren"], Command::Close(')')),
];

const PORTUGUESE: &[(&[&str], Command)] = &[
    (&["novo", "paragrafo"], PARAGRAPH),
    (&["nova", "linha"], LINE),
    (&["quebra", "de", "linha"], LINE),
    (&["abre", "aspas"], Command::Open('"')),
    (&["abrir", "aspas"], Command::Open('"')),
    (&["fecha", "aspas"], Command::Close('"')),
    (&["fechar", "aspas"], Command::Close('"')),
    (&["abre", "parenteses"], Command::Open('(')),
    (&["abre", "parentese"], Command::Open('(')),
    (&["abrir", "parenteses"], Command::Open('(')),
    (&["fecha", "parenteses"], Command::Close(')')),
    (&["fecha", "parentese"], Command::Close(')')),
    (&["fechar", "parenteses"], Command::Close(')')),
];

fn table(language: Language) -> &'static [(&'static [&'static str], Command)] {
    match language {
        Language::English => ENGLISH,
        Language::Portuguese => PORTUGUESE,
    }
}

/// `transcript` with the spoken formatting phrases of `languages` applied.
pub(super) fn apply(transcript: &str, languages: &[Language]) -> String {
    let commands: Vec<_> = languages
        .iter()
        .flat_map(|language| table(*language).iter())
        .collect();
    let quote_unquote = languages.contains(&Language::English);
    let chars: Vec<char> = transcript.chars().collect();
    let mut out = String::with_capacity(transcript.len());
    let mut index = 0;
    // Where the "unquote" closing an open "quote" starts.
    let mut pending_unquote: Option<usize> = None;
    while index < chars.len() {
        if pending_unquote == Some(index) {
            pending_unquote = None;
            index = emit(
                &mut out,
                Command::Close('"'),
                &chars,
                index + "unquote".len(),
            );
            continue;
        }
        if word_start(&chars, index) {
            let found = commands.iter().find_map(|(words, command)| {
                phrase_at(&chars, index, words).map(|end| (*command, end))
            });
            if let Some((command, end)) = found
                && !(matches!(command, Command::Break(_)) && names_something(&chars, end))
            {
                index = emit(&mut out, command, &chars, end);
                continue;
            }
            if quote_unquote
                && pending_unquote.is_none()
                && let Some(end) = phrase_at(&chars, index, &["quote"])
                && let Some(close) = unquote_after(&chars, end)
            {
                pending_unquote = Some(close);
                index = emit(&mut out, Command::Open('"'), &chars, end);
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// Whether a break phrase ending at `end` goes on to say what it is a line or paragraph of: "a
/// new line of products", "uma nova linha de produtos".
fn names_something(chars: &[char], end: usize) -> bool {
    let next = skip(chars, end, &[]);
    next > end
        && ["of", "de", "do", "da", "dos", "das"]
            .iter()
            .any(|word| phrase_at(chars, next, &[word]).is_some())
}

/// Where the first whole-word "unquote" after `from` starts, when some words come between:
/// "quote unquote experts" is a figure of speech, not a quote.
fn unquote_after(chars: &[char], from: usize) -> Option<usize> {
    let mut words_between = false;
    for index in from..chars.len() {
        if word_start(chars, index) && phrase_at(chars, index, &["unquote"]).is_some() {
            return words_between.then_some(index);
        }
        words_between |= fold(chars[index]).is_alphanumeric();
    }
    None
}

/// Writes `command` and returns where the text continues.
fn emit(out: &mut String, command: Command, chars: &[char], end: usize) -> usize {
    match command {
        Command::Break(breaks) => {
            trim_end(out, &[',', ';'], 0);
            out.push_str(breaks);
            skip(chars, end, &[',', '.', ';', ':'])
        }
        Command::Open(mark) => {
            trim_end(out, &[], 0);
            if out
                .chars()
                .next_back()
                .is_some_and(|last| !matches!(last, '\n' | '(' | '[' | '"'))
            {
                out.push(' ');
            }
            out.push(mark);
            skip(chars, end, &[',', '.'])
        }
        Command::Close(mark) => {
            trim_end(out, &[','], 0);
            out.push(mark);
            let next = skip(chars, end, &[]);
            if chars.get(next).is_some_and(|c| c.is_alphanumeric()) {
                out.push(' ');
            }
            next
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english(text: &str) -> String {
        apply(text, &[Language::English])
    }

    fn portuguese(text: &str) -> String {
        apply(text, &[Language::Portuguese])
    }

    #[test]
    fn english_breaks_replace_their_phrases_and_the_punctuation_around_them() {
        assert_eq!(english("Hello. New line. Next"), "Hello.\nNext");
        assert_eq!(english("One. New paragraph. Two."), "One.\n\nTwo.");
        assert_eq!(english("one, line break, two"), "one\ntwo");
        assert_eq!(english("a NEW LINE new line b"), "a\n\nb");
    }

    #[test]
    fn english_quotes_and_parentheses_become_marks() {
        assert_eq!(
            english("He said open quote hi close quote."),
            "He said \"hi\"."
        );
        assert_eq!(
            english("He said, Open quote, hello, close quote, and left."),
            "He said, \"hello\", and left."
        );
        assert_eq!(
            english("the plan open parenthesis draft close parenthesis is ready"),
            "the plan (draft) is ready"
        );
        assert_eq!(english("see open paren below close paren."), "see (below).");
        assert_eq!(
            english("He said quote hello unquote."),
            "He said \"hello\"."
        );
        for text in [
            "Can I get a quote for this",
            "quote unquote experts",
            "an unquote alone",
        ] {
            assert_eq!(english(text), text);
        }
    }

    #[test]
    fn portuguese_phrases_work_with_or_without_accents_and_in_any_case() {
        assert_eq!(portuguese("Olá. Novo parágrafo. Tchau."), "Olá.\n\nTchau.");
        assert_eq!(portuguese("Olá. novo paragrafo. Tchau."), "Olá.\n\nTchau.");
        assert_eq!(portuguese("um, nova linha, dois"), "um\ndois");
        assert_eq!(portuguese("um NOVA LINHA dois"), "um\ndois");
        assert_eq!(portuguese("um quebra de linha dois"), "um\ndois");
        assert_eq!(
            portuguese("Ele disse abre aspas oi fecha aspas e saiu"),
            "Ele disse \"oi\" e saiu"
        );
        assert_eq!(
            portuguese("Ele é, abre aspas, especialista, fecha aspas."),
            "Ele é, \"especialista\"."
        );
        assert_eq!(
            portuguese("o plano abre parênteses rascunho fecha parênteses está pronto"),
            "o plano (rascunho) está pronto"
        );
        assert_eq!(
            portuguese("veja Abre Parêntese abaixo Fecha Parêntese."),
            "veja (abaixo)."
        );
    }

    #[test]
    fn other_words_are_left_alone() {
        for text in [
            "aspas",
            "uma linha nova",
            "a nova linhagem",
            "dois parágrafos novos",
            "abre a porta e fecha a janela",
            "The newline character",
            "a new liner",
            "line breaker",
            "open quotes",
            "Plain text, nothing to do.",
            "",
        ] {
            assert_eq!(english(text), text);
            assert_eq!(portuguese(text), text);
        }
        assert_eq!(
            english("We launched a new line of products"),
            "We launched a new line of products"
        );
        assert_eq!(
            portuguese("Lançamos uma nova linha de produtos e um novo parágrafo da lei"),
            "Lançamos uma nova linha de produtos e um novo parágrafo da lei"
        );
        assert_eq!(portuguese("um. Nova linha. De novo"), "um.\nDe novo");
        // "quote" … "unquote" is English only.
        assert_eq!(portuguese("quote oi unquote"), "quote oi unquote");
    }
}
