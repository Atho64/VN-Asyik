//! Full-width to half-width folding shared by the text normalisers.

/// Map one full-width ASCII form (U+FF01..=U+FF5E) onto its half-width
/// counterpart; any other character passes through unchanged.
///
/// Siglus captions write `Ｄａｙ １` and `７月２６日` with full-width letters
/// and digits, so a parser that skips this mapping reads nothing. This is the
/// one copy of the arithmetic; `search` and `window_title` both use it.
#[must_use]
pub(crate) fn half_width_form(ch: char) -> char {
    if ('\u{FF01}'..='\u{FF5E}').contains(&ch) {
        char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch)
    } else {
        ch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_width_letters_digits_and_punctuation_fold() {
        assert_eq!(half_width_form('Ｄ'), 'D');
        assert_eq!(half_width_form('１'), '1');
        assert_eq!(half_width_form('！'), '!');
        assert_eq!(half_width_form('Ａ'), 'A');
    }

    #[test]
    fn other_characters_pass_through() {
        assert_eq!(half_width_form('月'), '月');
        assert_eq!(half_width_form('a'), 'a');
        assert_eq!(half_width_form('\u{3000}'), '\u{3000}');
    }
}
