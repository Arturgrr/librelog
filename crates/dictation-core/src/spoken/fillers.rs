//! Hesitations a speech model writes down verbatim leave the dictation. A filler matches as a
//! whole word, lowercase or with only its first letter capitalised, so "umbrella", "Uhura",
//! "uh-huh", "ER" and "é" the verb stay. The comma or ellipsis that comes with a filler goes
//! with it, the spaces around it close up, and a capitalised filler that started a sentence
//! hands its capital to the next word. Line breaks are kept, and a filler asked as a question
//! ("Hã?") is kept.

use super::{Language, is_space, ordinary_in, trim_end};

#[derive(Clone, Copy)]
enum Filler {
    /// Exactly this word.
    Word(&'static str),
    /// This word, its last letter held as long as the speaker held it: "um", "umm", "ummm".
    Held(&'static str),
    /// This word trailing off into an ellipsis at the start of a sentence: "É... eu acho".
    /// Anywhere else it is a word ("Ele é... complicado").
    TrailingOff(&'static str),
}

/// English, from Hex: "er" and "erm" are never held, so "err" the verb stays.
const ENGLISH: &[Filler] = &[
    Filler::Held("um"),
    Filler::Held("uh"),
    Filler::Word("erm"),
    Filler::Word("er"),
    Filler::Held("ah"),
];

/// Portuguese, kept to sounds that are never words: "é" only held ("éé") or trailing off at
/// the start of a sentence, and never "tipo", "então", "né" or "assim", which often mean
/// something.
const PORTUGUESE: &[Filler] = &[
    Filler::Held("éé"),
    Filler::TrailingOff("é"),
    Filler::Held("hã"),
    Filler::Word("ãh"),
    Filler::Word("ahn"),
    Filler::Word("ãhn"),
    Filler::Held("hum"),
    Filler::Held("hm"),
];

fn table(language: Language) -> &'static [Filler] {
    match language {
        Language::English => ENGLISH,
        Language::Portuguese => PORTUGUESE,
    }
}

/// `transcript` without the fillers of `languages`.
pub(super) fn apply(transcript: &str, languages: &[Language]) -> String {
    let fillers: Vec<Filler> = languages
        .iter()
        .flat_map(|language| table(*language).iter().copied())
        .collect();
    let chars: Vec<char> = transcript.chars().collect();
    let mut out = String::with_capacity(transcript.len());
    // A filler was just removed: the next text decides the spacing.
    let mut removed = false;
    let mut capitalize = false;
    let mut index = 0;
    while index < chars.len() {
        if let Some(end) = filler_at(&chars, index, &fillers, languages, sentence_start(&out)) {
            trim_end(&mut out, &[], 0);
            if !removed {
                capitalize = chars[index].is_uppercase() && sentence_start(&out);
            }
            index = after_filler(&chars, end);
            removed = true;
            continue;
        }
        let c = chars[index];
        index += 1;
        if removed {
            if is_space(c) {
                continue;
            }
            if c == '\n' {
                removed = false;
                capitalize = false;
                out.push(c);
                continue;
            }
            if matches!(c, '.' | '!' | ',') && sentence_start(&out) {
                continue;
            }
            removed = false;
            let closing = matches!(
                c,
                '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '”' | '’' | '…'
            ) || (c == '"'
                && chars.get(index).is_none_or(|next| !next.is_alphanumeric()));
            if closing {
                trim_end(&mut out, &[','], 0);
                capitalize = false;
            } else {
                if needs_space(&out) {
                    out.push(' ');
                }
                if std::mem::take(&mut capitalize) {
                    out.extend(c.to_uppercase());
                    continue;
                }
            }
        }
        out.push(c);
    }
    if removed {
        trim_end(&mut out, &[','], 0);
    }
    out
}

fn joins_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '\'' | '’')
}

/// Where the filler starting at `start` ends, if one does.
fn filler_at(
    chars: &[char],
    start: usize,
    fillers: &[Filler],
    languages: &[Language],
    at_sentence_start: bool,
) -> Option<usize> {
    if start > 0 && joins_word(chars[start - 1]) {
        return None;
    }
    let end = start
        + chars[start..]
            .iter()
            .take_while(|c| joins_word(**c))
            .count();
    let word = &chars[start..end];
    let (first, rest) = word.split_first()?;
    if !rest.iter().all(|c| c.is_lowercase()) || !first.is_alphabetic() {
        return None;
    }
    let lower: String = word.iter().flat_map(|c| c.to_lowercase()).collect();
    if ordinary_in(languages, &lower) {
        return None;
    }
    let next = super::skip(chars, end, &[]);
    if chars.get(next) == Some(&'?') {
        return None;
    }
    let trails_off =
        chars.get(next) == Some(&'…') || chars.get(next..next + 3) == Some(&['.', '.', '.'][..]);
    fillers
        .iter()
        .any(|filler| match *filler {
            Filler::Word(filler) => lower == filler,
            Filler::Held(filler) => held(&lower, filler),
            Filler::TrailingOff(filler) => lower == filler && trails_off && at_sentence_start,
        })
        .then_some(end)
}

/// Whether `word` is `filler` with its last letter held for as long as the speaker held it.
fn held(word: &str, filler: &str) -> bool {
    let Some(rest) = word.strip_prefix(filler) else {
        return false;
    };
    let last = filler.chars().next_back();
    rest.chars().all(|c| Some(c) == last)
}

/// Where the text continues after a filler ending at `end`, past its own comma or ellipsis.
fn after_filler(chars: &[char], end: usize) -> usize {
    let mut index = super::skip(chars, end, &[]);
    match chars.get(index) {
        Some(',' | '…') => index += 1,
        Some('.') if chars.get(index + 1) == Some(&'.') && chars.get(index + 2) == Some(&'.') => {
            index += 3
        }
        _ => {}
    }
    super::skip(chars, index, &[])
}

fn sentence_start(out: &str) -> bool {
    out.trim_end_matches(is_space)
        .chars()
        .next_back()
        .is_none_or(|last| matches!(last, '.' | '!' | '?' | '\n'))
}

fn needs_space(out: &str) -> bool {
    let mut reversed = out.chars().rev();
    match reversed.next() {
        None | Some('\n' | '(' | '[' | '{' | '“' | '‘') => false,
        Some('"') => reversed
            .next()
            .is_some_and(|before| !before.is_whitespace()),
        Some(_) => true,
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
    fn english_fillers_leave_with_their_commas_and_spaces() {
        assert_eq!(english("So, um, we should go."), "So, we should go.");
        assert_eq!(english("I think uh we should go."), "I think we should go.");
        assert_eq!(english("I think, um."), "I think.");
        assert_eq!(english("Well umm... maybe"), "Well maybe");
        assert_eq!(english("It was, er, erm, fine"), "It was, fine");
        assert_eq!(english("Done, uhhh"), "Done");
        assert_eq!(english("(um, aside)"), "(aside)");
    }

    #[test]
    fn a_filler_that_started_a_sentence_hands_on_its_capital() {
        assert_eq!(english("Um, so I think"), "So I think");
        assert_eq!(
            english("Hello. Uh, we left. Ah. Then"),
            "Hello. We left. Then"
        );
        assert_eq!(english("Uh, um, okay"), "Okay");
        assert_eq!(portuguese("Éé, vamos lá"), "Vamos lá");
    }

    #[test]
    fn line_breaks_are_kept() {
        assert_eq!(english("First line.\nUm, second"), "First line.\nSecond");
        assert_eq!(english("one um\ntwo"), "one\ntwo");
    }

    #[test]
    fn english_words_that_look_like_fillers_are_left_alone() {
        for text in [
            "The umbrella is ahead of the error.",
            "Uhura said uh-huh.",
            "Take him to the ER.",
            "UM is a university.",
            "To err is human.",
            "drum, Ahab, ermine",
            "Uh? What did you say",
            "Plain text, nothing to do.",
            "",
        ] {
            assert_eq!(english(text), text);
        }
    }

    #[test]
    fn portuguese_hesitations_leave_but_the_verb_e_stays() {
        assert_eq!(portuguese("é... eu acho que sim"), "eu acho que sim");
        assert_eq!(portuguese("É... eu acho que sim"), "Eu acho que sim");
        assert_eq!(portuguese("Certo. É… eu acho"), "Certo. Eu acho");
        assert_eq!(portuguese("Eu acho, éé, que sim"), "Eu acho, que sim");
        assert_eq!(portuguese("Então ééé vamos"), "Então vamos");
        assert_eq!(portuguese("Hã, não sei"), "Não sei");
        assert_eq!(portuguese("Ahn, beleza"), "Beleza");
        assert_eq!(portuguese("Hmm, talvez amanhã"), "Talvez amanhã");
        assert_eq!(portuguese("Humm... pode ser"), "Pode ser");
        assert_eq!(portuguese("Vou, hum, pensar"), "Vou, pensar");
    }

    #[test]
    fn portuguese_words_are_left_alone() {
        for text in [
            "Ele é bom.",
            "É bom demais.",
            "É, eu acho que sim.",
            "Ele é... complicado.",
            "Então, é... difícil.",
            "Eu quero um café.",
            "Ah, entendi.",
            "Tipo assim, foi ótimo, né?",
            "Hã? Não entendi.",
            "humano, hábito, ahnn",
            "",
        ] {
            assert_eq!(portuguese(text), text);
        }
    }
}
