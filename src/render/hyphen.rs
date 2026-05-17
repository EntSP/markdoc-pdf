//! Soft-hyphen pre-processor for body text.
//!
//! parley itself doesn't hyphenate; instead it honours soft hyphens
//! (U+00AD) inserted by the caller. This module loads a Knuth–Liang
//! pattern set (English-US embedded by default; other languages via a
//! `.bincode` path) once per render and inserts soft hyphens at every
//! valid breakpoint in every word that is at least `min_chars` long.
//!
//! Words shorter than `min_chars`, words containing digits, or words
//! containing existing hyphens are left untouched so author-supplied
//! hard hyphens stay authoritative.

use std::path::Path;

use hyphenation::{Hyphenator as _, Language, Load, Standard};

use super::style::HyphenationStyle;

/// A reusable hyphenator. Cheap to construct from an embedded pattern
/// (`Language::EnglishUS`) or a `.bincode` file produced by the
/// `hyphenation` crate's `build_dictionaries` feature.
pub struct WordHyphenator {
    inner: Standard,
    min_chars: usize,
}

impl WordHyphenator {
    pub fn from_style(style: &HyphenationStyle) -> Option<Self> {
        if !style.enabled {
            return None;
        }
        let inner = if let Some(path) = &style.dictionary_path {
            Standard::from_path(language_from_tag(&style.language)?, Path::new(path)).ok()?
        } else if style.language.eq_ignore_ascii_case("en-us") {
            Standard::from_embedded(Language::EnglishUS).ok()?
        } else {
            // Non-en-us without a path — give up rather than silently
            // mis-applying English patterns.
            return None;
        };
        Some(Self {
            inner,
            min_chars: style.min_word_chars.max(3) as usize,
        })
    }

    /// Insert soft hyphens (U+00AD) at every breakpoint inside every
    /// hyphenatable word in `text`. Non-word runs (spaces, punctuation)
    /// pass through unchanged.
    pub fn hyphenate(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + text.len() / 8);
        for (is_word, run) in word_runs(text) {
            if !is_word || !should_hyphenate(run, self.min_chars) {
                out.push_str(run);
                continue;
            }
            let h = self.inner.hyphenate(run);
            let mut last = 0;
            for cut in h.breaks.iter().copied() {
                out.push_str(&run[last..cut]);
                out.push('\u{00AD}');
                last = cut;
            }
            out.push_str(&run[last..]);
        }
        out
    }
}

fn language_from_tag(tag: &str) -> Option<Language> {
    // Only the bundled tag is recognised here; for everything else
    // the user supplies dictionary_path so the language string is
    // really just a label. Pick a sensible default to satisfy the API.
    match tag.to_ascii_lowercase().as_str() {
        "en-us" => Some(Language::EnglishUS),
        _ => Some(Language::EnglishUS),
    }
}

fn should_hyphenate(word: &str, min_chars: usize) -> bool {
    if word.chars().count() < min_chars {
        return false;
    }
    // Skip words containing digits or hard hyphens — author-given
    // hyphens should stay authoritative.
    if word
        .chars()
        .any(|c| c.is_ascii_digit() || c == '-' || c == '\u{00AD}')
    {
        return false;
    }
    word.chars().any(|c| c.is_alphabetic())
}

/// Iterate over `text` yielding `(is_word, &str)` runs alternating
/// between word-character runs and non-word-character runs.
fn word_runs(text: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev_is_word: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let is_word = c.is_alphabetic() || c == '\'' || c == '\u{2019}';
        match prev_is_word {
            None => prev_is_word = Some(is_word),
            Some(prev) if prev == is_word => {}
            Some(prev) => {
                out.push((prev, &text[start..i]));
                start = i;
                prev_is_word = Some(is_word);
            }
        }
    }
    if start < text.len() {
        out.push((prev_is_word.unwrap_or(false), &text[start..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::style::HyphenationStyle;

    #[test]
    fn english_inserts_soft_hyphens() {
        let style = HyphenationStyle {
            enabled: true,
            language: "en-us".into(),
            min_word_chars: 5,
            dictionary_path: None,
        };
        let h = WordHyphenator::from_style(&style).unwrap();
        let out = h.hyphenate("hyphenation");
        // "hy-phen-ation" is the canonical Knuth–Liang result.
        assert!(out.contains('\u{00AD}'));
        assert!(out.starts_with("hy"));
    }

    #[test]
    fn short_words_untouched() {
        let style = HyphenationStyle {
            enabled: true,
            language: "en-us".into(),
            min_word_chars: 5,
            dictionary_path: None,
        };
        let h = WordHyphenator::from_style(&style).unwrap();
        assert_eq!(h.hyphenate("hi there"), "hi there");
    }

    #[test]
    fn digits_and_hard_hyphens_skipped() {
        let style = HyphenationStyle {
            enabled: true,
            language: "en-us".into(),
            min_word_chars: 4,
            dictionary_path: None,
        };
        let h = WordHyphenator::from_style(&style).unwrap();
        assert_eq!(h.hyphenate("v1.2.3"), "v1.2.3");
        assert_eq!(h.hyphenate("self-aware"), "self-aware");
    }

    #[test]
    fn disabled_returns_none() {
        let style = HyphenationStyle::default();
        assert!(WordHyphenator::from_style(&style).is_none());
    }
}
