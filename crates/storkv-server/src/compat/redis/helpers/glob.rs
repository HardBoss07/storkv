//! Glob matching and pattern validation.

use super::super::codec::Resp;

/// Maximum accepted `MATCH` or `KEYS` pattern length in bytes.
pub(crate) const MAX_GLOB_PATTERN_LEN: usize = 1024;

/// Glob matcher supporting `*`, `?`, and `[abc]`/`[a-z]` classes.
///
/// The matcher is iterative and uses one backtrack point for the last `*`.
/// Its worst-case cost is `O(pattern_len * text_len)` and it uses no recursion.
pub(crate) fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    let mut pattern_index = 0usize;
    let mut text_index = 0usize;
    let mut star_pattern: Option<usize> = None;
    let mut star_text = 0usize;

    while text_index < text.len() {
        if pattern_index < pattern.len() {
            let token = pattern[pattern_index];
            match token {
                b'*' => {
                    star_pattern = Some(pattern_index);
                    star_text = text_index;
                    pattern_index += 1;
                    continue;
                }
                b'?' => {
                    pattern_index += 1;
                    text_index += 1;
                    continue;
                }
                b'[' => {
                    if let Some((matched, after)) =
                        match_class(&pattern[pattern_index..], &text[text_index..])
                    {
                        if matched {
                            let consumed = pattern.len() - pattern_index - after.len();
                            pattern_index += consumed;
                            text_index += 1;
                            continue;
                        }
                    }
                }
                b'\\' => {
                    let (escaped, advance) = match pattern.get(pattern_index + 1) {
                        Some(&escaped) => (escaped, 2),
                        None => (b'\\', 1),
                    };
                    if text[text_index] == escaped {
                        pattern_index += advance;
                        text_index += 1;
                        continue;
                    }
                }
                literal => {
                    if text[text_index] == literal {
                        pattern_index += 1;
                        text_index += 1;
                        continue;
                    }
                }
            }
        }
        let Some(star) = star_pattern else {
            return false;
        };
        star_text += 1;
        text_index = star_text;
        pattern_index = star + 1;
    }

    while pattern.get(pattern_index) == Some(&b'*') {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

/// Reject a glob pattern that is larger than the accepted limit.
pub(crate) fn validate_glob_pattern(pattern: &[u8]) -> Result<(), Resp> {
    if pattern.len() > MAX_GLOB_PATTERN_LEN {
        return Err(Resp::error("ERR pattern exceeds maximum length"));
    }
    Ok(())
}

fn match_class<'a>(pattern: &'a [u8], text: &[u8]) -> Option<(bool, &'a [u8])> {
    let target = *text.first()?;
    let mut index = 1;
    let mut matched = false;
    let negated = pattern.get(index) == Some(&b'^');
    if negated {
        index += 1;
    }
    let mut closed = false;
    while index < pattern.len() {
        let byte = pattern[index];
        if byte == b']' && index > 1 {
            closed = true;
            index += 1;
            break;
        }
        if pattern.get(index + 1) == Some(&b'-') {
            if let Some(end) = pattern.get(index + 2) {
                if target >= byte && target <= *end {
                    matched = true;
                }
                index += 3;
                continue;
            }
        }
        if target == byte {
            matched = true;
        }
        index += 1;
    }
    if !closed {
        return None;
    }
    Some((matched != negated, pattern.get(index..).unwrap_or(&[])))
}
