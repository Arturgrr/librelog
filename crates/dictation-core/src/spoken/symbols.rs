//! Spoken symbols become the characters, joined to their neighbours as written text joins
//! them: "hashtag release" is "#release", "minha underline variável" is "minha_variável", and
//! "acme ponto com" is "acme.com". Matching ignores case and accents and needs word
//! boundaries, so "hashtags" or "arrobas" stay as they are. The commas, periods and spaces a
//! model puts around a spoken symbol go with it.
//!
//! The tables keep to symbols whose words are rarely said for anything else. Punctuation said
//! out loud ("comma", "vírgula", "dois pontos", "ponto final") is left to the speech model and
//! AI cleanup: "dois pontos" is also "two points", "vírgula" is the decimal comma in "dois
//! vírgula cinco", and "ponto" alone is everywhere.

use super::{Language, ordinary_in, phrase_at, skip, trim_end, word_start};

/// Which neighbour a symbol attaches to.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Join {
    /// The next word: "#release".
    Next,
    /// Both words: "my_variable".
    Both,
    /// The previous word, leaving what follows as it was: "acme.com".
    Previous,
}

type Symbol = (&'static [&'static str], &'static str, Join);

const ENGLISH: &[Symbol] = &[
    (&["hashtag"], "#", Join::Next),
    (&["hash", "sign"], "#", Join::Next),
    (&["at", "sign"], "@", Join::Next),
    (&["underscore"], "_", Join::Both),
    (&["dot", "com"], ".com", Join::Previous),
];

const PORTUGUESE: &[Symbol] = &[
    (&["arroba"], "@", Join::Next),
    (&["hashtag"], "#", Join::Next),
    (&["cerquilha"], "#", Join::Next),
    (&["underline"], "_", Join::Both),
    (&["underscore"], "_", Join::Both),
    (&["ponto", "com"], ".com", Join::Previous),
    (&["ponto", "br"], ".br", Join::Previous),
];

fn table(language: Language) -> &'static [Symbol] {
    match language {
        Language::English => ENGLISH,
        Language::Portuguese => PORTUGUESE,
    }
}

/// `transcript` with the spoken symbols of `languages` applied.
pub(super) fn apply(transcript: &str, languages: &[Language]) -> String {
    let symbols: Vec<&Symbol> = languages
        .iter()
        .flat_map(|language| table(*language).iter())
        .filter(|(words, _, _)| !(words.len() == 1 && ordinary_in(languages, words[0])))
        .collect();
    let chars: Vec<char> = transcript.chars().collect();
    let mut out = String::with_capacity(transcript.len());
    // Where the last symbol written ends, so trimming never removes it.
    let mut floor = 0;
    let mut index = 0;
    while index < chars.len() {
        if word_start(&chars, index)
            && let Some((symbol, join, end)) = symbols.iter().find_map(|(words, symbol, join)| {
                phrase_at(&chars, index, words).map(|end| (*symbol, *join, end))
            })
        {
            if join != Join::Next {
                trim_end(&mut out, &[','], floor);
            }
            out.push_str(symbol);
            floor = out.len();
            index = match join {
                Join::Previous => end,
                Join::Next | Join::Both => skip(&chars, end, &[',', '.']),
            };
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
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
    fn english_symbols_join_their_neighbours() {
        assert_eq!(english("hashtag release"), "#release");
        assert_eq!(english("Hashtag, release"), "#release");
        assert_eq!(english("my underscore variable"), "my_variable");
        assert_eq!(english("tag it hash sign release"), "tag it #release");
        assert_eq!(english("mail me at sign home"), "mail me @home");
        assert_eq!(english("go to example dot com."), "go to example.com.");
        assert_eq!(
            english("go to example, dot com, now"),
            "go to example.com, now"
        );
        assert_eq!(english("a underscore underscore b"), "a__b");
    }

    #[test]
    fn portuguese_symbols_join_their_neighbours_with_or_without_accents() {
        assert_eq!(portuguese("me segue no arroba joao"), "me segue no @joao");
        assert_eq!(portuguese("Arroba, joao"), "@joao");
        assert_eq!(portuguese("usa a hashtag lançamento"), "usa a #lançamento");
        assert_eq!(portuguese("cerquilha urgente"), "#urgente");
        assert_eq!(portuguese("minha underline variável"), "minha_variável");
        assert_eq!(portuguese("minha underscore variável"), "minha_variável");
        assert_eq!(portuguese("acesse acme ponto com"), "acesse acme.com");
        assert_eq!(
            portuguese("acesse acme Ponto Com Ponto BR, por favor"),
            "acesse acme.com.br, por favor"
        );
        assert_eq!(
            portuguese("joao arroba acme ponto com ponto br"),
            "joao @acme.com.br"
        );
    }

    #[test]
    fn other_words_are_left_alone() {
        for text in [
            "hashtags everywhere",
            "dez arrobas de carne",
            "o ponto de vista",
            "ponto combinado",
            "dois pontos no jogo",
            "dois vírgula cinco",
            "look at Mina's sign",
            "dot comma",
            "the colonel",
            "Plain text, nothing to do.",
            "",
        ] {
            assert_eq!(english(text), text);
            assert_eq!(portuguese(text), text);
        }
        assert_eq!(english("underline this"), "underline this");
    }
}
