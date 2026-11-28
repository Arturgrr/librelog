//! The context prompt Whisper and Qwen3-ASR read the dictation vocabulary
//! from: the first terms in priority order, comma-separated so the echo
//! filter in `session` can recognise the list, and as many as fit the
//! model's prompt slot. Other families (Parakeet, Cohere, GigaAM, Nemotron)
//! have no slot for it and get no prompt.
//!
//! Adapted from anomalyco/hex at 96d8ed31 (`src/dictation/vocabulary.rs`,
//! `src/transcription/gguf_session.rs`).

/// The most tokens of terms a model of `architecture` reads. Whisper's
/// previous-text slot holds 223 tokens and transcribe.cpp keeps its last
/// ones when the prompt overflows, which would drop the most important terms
/// first, so the list stays well inside it. Qwen3-ASR's system message
/// shares the context window with the audio and the answer.
pub(crate) fn prompt_budget(architecture: &str) -> Option<usize> {
    match architecture {
        "whisper" => Some(192),
        "qwen3_asr" => Some(256),
        _ => None,
    }
}

/// The prompt for `terms`, most important first, as long as `tokens` says
/// fits `budget`; `None` without a usable term. Terms with the list's own
/// separators, or with bracketed text a tokenizer may read as a control
/// token (which fails the run), are left out.
pub(crate) fn prompt(
    terms: &[String],
    budget: usize,
    tokens: impl Fn(&str) -> usize,
) -> Option<String> {
    let mut kept: Vec<&str> = Vec::new();
    let mut prompt = String::new();
    for term in terms {
        let term = term.trim();
        if term.is_empty()
            || term.contains([',', ';', '\n', '<', '>', '[', ']'])
            || kept.iter().any(|seen| seen.eq_ignore_ascii_case(term))
        {
            continue;
        }
        let candidate = if prompt.is_empty() {
            term.to_string()
        } else {
            format!("{prompt}, {term}")
        };
        if tokens(&candidate) > budget {
            break;
        }
        kept.push(term);
        prompt = candidate;
    }
    (!prompt.is_empty()).then_some(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(list: &[&str]) -> Vec<String> {
        list.iter().map(|term| term.to_string()).collect()
    }

    fn chars(text: &str) -> usize {
        text.chars().count()
    }

    #[test]
    fn the_prompt_lists_the_first_terms_that_fit_the_budget() {
        let list = terms(&[
            "Librelog",
            " Ottobit ",
            "librelog",
            "a, b",
            "<|en|>",
            "Kubernetes",
        ]);
        assert_eq!(
            prompt(&list, 100, chars).as_deref(),
            Some("Librelog, Ottobit, Kubernetes")
        );
        assert_eq!(
            prompt(&list, "Librelog, Ottobit".len(), chars).as_deref(),
            Some("Librelog, Ottobit")
        );
        assert_eq!(prompt(&list, 3, chars), None);
        assert_eq!(prompt(&[], 100, chars), None);
        assert_eq!(prompt(&terms(&["  ", ""]), 100, chars), None);
    }

    #[test]
    fn only_whisper_and_qwen3_asr_read_a_prompt() {
        assert!(prompt_budget("whisper").is_some());
        assert!(prompt_budget("qwen3_asr").is_some());
        for architecture in ["parakeet", "cohere_asr", "gigaam", "nemotron"] {
            assert_eq!(prompt_budget(architecture), None, "{architecture}");
        }
    }
}
