//! Spoken commands applied to a finished transcript without a model: hesitations dropped,
//! spoken line breaks, quotes and parentheses written out, and spoken symbols joined to their
//! words. Each set is the person's own choice, and only the languages they dictate in bring
//! their words: English and Portuguese have tables, every other language is left as heard.
//! Ported from Hex; see THIRD_PARTY_NOTICES.md.

mod fillers;
mod formatting;
mod symbols;

use serde::{Deserialize, Serialize};

/// Which spoken commands apply to a dictation, and the languages it is dictated in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpokenRules {
    /// The dictation's languages as BCP 47 tags ("pt-BR", "en"); languages other than English
    /// and Portuguese bring no words.
    pub languages: Vec<String>,
    /// Drop hesitations such as "um" or "éé".
    pub fillers: bool,
    /// "new line", "abre aspas" and the like become line breaks, quotes and parentheses.
    pub formatting: bool,
    /// "at sign", "arroba", "underscore" and the like become symbols joined to their words.
    pub symbols: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    English,
    Portuguese,
}

impl Language {
    fn from_tag(tag: &str) -> Option<Self> {
        let base = tag.trim().split(['-', '_']).next()?.to_ascii_lowercase();
        match base.as_str() {
            "en" => Some(Self::English),
            "pt" => Some(Self::Portuguese),
            _ => None,
        }
    }

    /// Ordinary words of this language that sound like another language's command: "um" is
    /// Portuguese for "a", "hum", "underline" and "underscore" are English words.
    fn ordinary(self) -> &'static [&'static str] {
        match self {
            Self::English => &["hum", "underline", "underscore"],
            Self::Portuguese => &["um", "ah"],
        }
    }
}

impl SpokenRules {
    fn languages(&self) -> Vec<Language> {
        let mut found = Vec::new();
        for language in self
            .languages
            .iter()
            .filter_map(|tag| Language::from_tag(tag))
        {
            if !found.contains(&language) {
                found.push(language);
            }
        }
        found
    }

    /// Whether the text may now hold line breaks, quotes or symbols the speaker asked for.
    pub(crate) fn writes_marks(&self) -> bool {
        (self.formatting || self.symbols) && !self.languages().is_empty()
    }
}

/// Whether a word is an ordinary word in one of `languages`, so no rule may take it.
fn ordinary_in(languages: &[Language], word: &str) -> bool {
    languages
        .iter()
        .any(|language| language.ordinary().contains(&word))
}

/// `text` with the spoken commands `rules` asks for applied: hesitations first, then line
/// breaks, quotes and parentheses, then symbols.
pub fn apply_spoken_rules(text: &str, rules: &SpokenRules) -> String {
    let languages = rules.languages();
    let mut text = text.to_owned();
    if languages.is_empty() {
        return text;
    }
    if rules.fillers {
        text = fillers::apply(&text, &languages);
    }
    if rules.formatting {
        text = formatting::apply(&text, &languages);
    }
    if rules.symbols {
        text = symbols::apply(&text, &languages);
    }
    text
}

/// `c` lowercased with Portuguese and Spanish accents dropped, as the phrase tables are
/// written: "Parênteses" reads as "parenteses".
fn fold(c: char) -> char {
    let mut lower = c.to_lowercase();
    let lower = match (lower.next(), lower.next()) {
        (Some(single), None) => single,
        _ => c,
    };
    match lower {
        'á' | 'à' | 'â' | 'ã' => 'a',
        'é' | 'ê' => 'e',
        'í' => 'i',
        'ó' | 'ô' | 'õ' => 'o',
        'ú' | 'ü' => 'u',
        'ç' => 'c',
        other => other,
    }
}

fn is_space(c: char) -> bool {
    c.is_whitespace() && c != '\n'
}

fn word_start(chars: &[char], index: usize) -> bool {
    index == 0 || !chars[index - 1].is_alphanumeric()
}

/// Where `words`, said with spaces between them, end when they start at `start` as whole
/// words, ignoring case and accents.
fn phrase_at(chars: &[char], start: usize, words: &[&str]) -> Option<usize> {
    let mut position = start;
    for (number, word) in words.iter().enumerate() {
        if number > 0 {
            let spaces = chars[position..]
                .iter()
                .take_while(|c| is_space(**c))
                .count();
            if spaces == 0 {
                return None;
            }
            position += spaces;
        }
        for expected in word.chars() {
            match chars.get(position) {
                Some(&c) if fold(c) == expected => position += 1,
                _ => return None,
            }
        }
    }
    chars
        .get(position)
        .is_none_or(|next| !next.is_alphanumeric())
        .then_some(position)
}

/// Pops trailing spaces, and any of `also`, never below `floor` bytes.
fn trim_end(out: &mut String, also: &[char], floor: usize) {
    while out.len() > floor
        && let Some(last) = out.chars().next_back()
        && (is_space(last) || also.contains(&last))
    {
        out.pop();
    }
}

fn skip(chars: &[char], mut index: usize, also: &[char]) -> usize {
    while chars
        .get(index)
        .is_some_and(|&c| is_space(c) || also.contains(&c))
    {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(languages: &[&str]) -> SpokenRules {
        SpokenRules {
            languages: languages.iter().map(|tag| (*tag).to_owned()).collect(),
            fillers: true,
            formatting: true,
            symbols: true,
        }
    }

    #[test]
    fn each_set_runs_only_when_turned_on() {
        let text = "Um, write to mina at sign home new line thanks";
        assert_eq!(
            apply_spoken_rules(text, &rules(&["en-US"])),
            "Write to mina @home\nthanks"
        );
        let only = |fillers, formatting, symbols| SpokenRules {
            languages: vec!["en".into()],
            fillers,
            formatting,
            symbols,
        };
        assert_eq!(
            apply_spoken_rules(text, &only(true, false, false)),
            "Write to mina at sign home new line thanks"
        );
        assert_eq!(
            apply_spoken_rules(text, &only(false, true, false)),
            "Um, write to mina at sign home\nthanks"
        );
        assert_eq!(
            apply_spoken_rules(text, &only(false, false, true)),
            "Um, write to mina @home new line thanks"
        );
        assert_eq!(apply_spoken_rules(text, &only(false, false, false)), text);
    }

    #[test]
    fn languages_without_a_table_keep_the_words_as_heard() {
        for languages in [&[][..], &["de"], &["es-ES", "fr"], &["  "]] {
            let text = "Um, er kommt new line arroba";
            assert_eq!(apply_spoken_rules(text, &rules(languages)), text);
        }
        assert!(!rules(&["de"]).writes_marks());
        assert!(rules(&["pt_BR"]).writes_marks());
    }

    #[test]
    fn portuguese_dictation_uses_its_own_words() {
        assert_eq!(
            apply_spoken_rules(
                "É... eu acho que o e-mail é joao arroba acme ponto com. Nova linha. Abraço",
                &rules(&["pt-BR"])
            ),
            "Eu acho que o e-mail é joao@acme.com.\nAbraço"
        );
    }

    #[test]
    fn a_word_ordinary_in_another_dictated_language_is_kept() {
        // "um" is Portuguese for "a", so with both languages it stays.
        assert_eq!(
            apply_spoken_rules("Eu quero um café", &rules(&["en", "pt"])),
            "Eu quero um café"
        );
        assert_eq!(
            apply_spoken_rules("Um, send it", &rules(&["en"])),
            "Send it"
        );
        // "hum" and "underline" are English words.
        assert_eq!(
            apply_spoken_rules("hum, I underline this", &rules(&["pt", "en"])),
            "hum, I underline this"
        );
        assert_eq!(
            apply_spoken_rules("Hum, minha underline variável", &rules(&["pt"])),
            "Minha_variável"
        );
    }
}
