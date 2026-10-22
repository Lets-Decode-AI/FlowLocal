use crate::store::DictEntry;

/// Context about the insertion point. `None` = unknown (no leading space).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InsertContext {
    pub prev_char: Option<char>,
}

/// Tidy raw STT text before it is pasted at the cursor.
pub fn postprocess(raw: &str, dict: &[DictEntry], ctx: InsertContext) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return String::new();
    }
    let (text, first_replaced) = apply_dictionary(&collapsed, dict);
    let text = if first_replaced {
        text
    } else {
        fix_first_letter(&text, ctx.prev_char)
    };
    match ctx.prev_char {
        Some(c) if needs_leading_space(c) => format!(" {text}"),
        _ => text,
    }
}

fn needs_leading_space(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '(' | '[' | '{' | '"' | '\'' | '/' | '-')
}

fn is_boundary(c: Option<&char>) -> bool {
    c.is_none_or(|c| !c.is_alphanumeric())
}

fn chars_eq_ignore_case(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Returns the rewritten text and whether a replacement started at index 0.
fn apply_dictionary(text: &str, dict: &[DictEntry]) -> (String, bool) {
    // Sorted longest term first so longer matches win over shorter overlapping ones.
    let mut entries: Vec<(Vec<char>, String)> = dict
        .iter()
        .filter_map(|d| {
            let term = d.term.split_whitespace().collect::<Vec<_>>().join(" ");
            if term.is_empty() {
                return None;
            }
            let out = d.replacement.as_deref().unwrap_or(&term).to_string();
            Some((term.chars().collect(), out))
        })
        .collect();
    entries.sort_by_key(|a| std::cmp::Reverse(a.0.len()));

    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut first_replaced = false;
    let mut i = 0;
    while i < chars.len() {
        let at_boundary = i == 0 || is_boundary(chars.get(i - 1));
        let hit = if at_boundary {
            entries.iter().find(|(term, _)| {
                let end = i + term.len();
                end <= chars.len()
                    && is_boundary(chars.get(end))
                    && chars[i..end]
                        .iter()
                        .zip(term)
                        .all(|(a, b)| chars_eq_ignore_case(*a, *b))
            })
        } else {
            None
        };
        match hit {
            Some((term, rep)) => {
                out.push_str(rep);
                first_replaced |= i == 0 && !rep.is_empty();
                i += term.len();
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    (
        out.split_whitespace().collect::<Vec<_>>().join(" "),
        first_replaced,
    )
}

fn fix_first_letter(text: &str, prev: Option<char>) -> String {
    let Some(first) = text.chars().next() else {
        return String::new();
    };
    let rest = &text[first.len_utf8()..];
    let upper = match prev {
        None => true,
        Some(' ' | '\t') => return text.to_string(),
        Some(c) if c.is_whitespace() || matches!(c, '.' | '?' | '!') => true,
        Some(c) if c.is_alphanumeric() || matches!(c, ',' | ';' | ':') => false,
        Some(_) => return text.to_string(),
    };
    if upper {
        first.to_uppercase().chain(rest.chars()).collect()
    } else if keeps_case(text) {
        text.to_string()
    } else {
        first.to_lowercase().chain(rest.chars()).collect()
    }
}

/// First word is an acronym (>= 2 uppercase letters) or the pronoun "I" / "I'm" / "I've"...
fn keeps_case(text: &str) -> bool {
    let word = text.split_whitespace().next().unwrap_or("");
    word.chars().filter(|c| c.is_uppercase()).count() >= 2
        || word.starts_with("I'")
        || word.starts_with("I\u{2019}")
        || word.trim_end_matches(|c: char| !c.is_alphanumeric()) == "I"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(term: &str, rep: Option<&str>) -> DictEntry {
        DictEntry {
            id: 0,
            term: term.into(),
            replacement: rep.map(String::from),
        }
    }
    fn ctx(c: Option<char>) -> InsertContext {
        InsertContext { prev_char: c }
    }

    #[test]
    fn collapses_whitespace_and_trims() {
        assert_eq!(
            postprocess("  hello \n  world  ", &[], ctx(None)),
            "Hello world"
        );
        assert_eq!(postprocess("   \n ", &[], ctx(Some('a'))), "");
    }

    #[test]
    fn capitalizes_first_letter() {
        assert_eq!(postprocess("hello there.", &[], ctx(None)), "Hello there.");
        assert_eq!(postprocess("hello", &[], ctx(Some('\n'))), "Hello");
    }

    #[test]
    fn dictionary_replaces_whole_word_case_insensitive() {
        let d = [e("open ai", Some("OpenAI"))];
        assert_eq!(
            postprocess("I use Open AI daily", &d, ctx(None)),
            "I use OpenAI daily"
        );
        assert_eq!(
            postprocess("the opener and open ais", &d, ctx(None)),
            "The opener and open ais"
        );
    }

    #[test]
    fn dictionary_term_without_replacement_fixes_casing() {
        let d = [e("Kubernetes", None)];
        assert_eq!(
            postprocess("deploy to kubernetes now", &d, ctx(None)),
            "Deploy to Kubernetes now"
        );
    }

    #[test]
    fn longer_terms_win_and_replaced_first_letter_kept() {
        let d = [
            e("open", Some("X")),
            e("open ai", Some("OpenAI")),
            e("iPhone", None),
        ];
        assert_eq!(postprocess("open ai rocks", &d, ctx(None)), "OpenAI rocks");
        assert_eq!(
            postprocess("iphone is nice", &d, ctx(None)),
            "iPhone is nice"
        );
        assert_eq!(
            postprocess("iphone is nice", &d, ctx(Some('.'))),
            " iPhone is nice"
        );
    }

    #[test]
    fn adds_leading_space_after_word_char() {
        assert_eq!(postprocess("hello.", &[], ctx(Some('.'))), " Hello.");
        assert_eq!(postprocess("world", &[], ctx(Some('d'))), " world");
    }

    #[test]
    fn no_leading_space_after_newline_or_space() {
        assert_eq!(postprocess("hello", &[], ctx(Some('\n'))), "Hello");
        assert_eq!(postprocess("hello", &[], ctx(Some('\r'))), "Hello");
        assert_eq!(postprocess("hello", &[], ctx(Some('('))), "hello");
        assert_eq!(postprocess("hello", &[], ctx(Some('"'))), "hello");
    }

    #[test]
    fn lowercases_first_letter_mid_sentence() {
        assert_eq!(
            postprocess("Then we go.", &[], ctx(Some('a'))),
            " then we go."
        );
        assert_eq!(postprocess("Then", &[], ctx(Some(','))), " then");
    }

    #[test]
    fn keeps_acronyms_and_pronoun_mid_sentence() {
        assert_eq!(
            postprocess("NASA said so", &[], ctx(Some('a'))),
            " NASA said so"
        );
        assert_eq!(
            postprocess("I think so", &[], ctx(Some('a'))),
            " I think so"
        );
        assert_eq!(postprocess("I'm here", &[], ctx(Some(','))), " I'm here");
        assert_eq!(postprocess("I've gone", &[], ctx(Some('a'))), " I've gone");
    }

    #[test]
    fn space_prev_keeps_stt_casing() {
        assert_eq!(
            postprocess("Then we go.", &[], ctx(Some(' '))),
            "Then we go."
        );
        assert_eq!(
            postprocess("then we go.", &[], ctx(Some(' '))),
            "then we go."
        );
        assert_eq!(postprocess("then", &[], ctx(Some('\t'))), "then");
    }

    #[test]
    fn empty_replacement_leaves_no_double_space() {
        let d = [e("um", Some("")), e("new   york", None)];
        assert_eq!(postprocess("so um we", &d, ctx(None)), "So we");
        assert_eq!(
            postprocess("go to New York now", &d, ctx(None)),
            "Go to new york now"
        );
    }

    #[test]
    fn unicode_text_and_terms_do_not_panic() {
        let d = [e("café", Some("Café"))];
        assert_eq!(
            postprocess("élan 日本語 🎉 café", &d, ctx(Some('é'))),
            " élan 日本語 🎉 Café"
        );
    }
}
