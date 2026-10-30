//! Sort keys that put titles in natural order.

/// Every run of ASCII digits is left-padded to this width. Twenty digits hold
/// any `u64`, so numbers up to that size compare by value.
const DIGIT_WIDTH: usize = 20;

/// Returns a key whose byte order is the natural order of `title`.
///
/// The title is lowercased and each run of ASCII digits is padded with leading
/// zeros, so "Vol.2" sorts before "Vol.10". Computing the key once per title
/// lets the index sort with plain byte comparisons instead of re-tokenizing
/// both strings on every comparison, which is what made LRR's title sort slow.
pub(crate) fn natural_key(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut key = String::with_capacity(lower.len() + DIGIT_WIDTH);
    let mut digits = String::new();
    for c in lower.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            push_padded(&mut key, &mut digits);
            key.push(c);
        }
    }
    push_padded(&mut key, &mut digits);
    key
}

fn push_padded(key: &mut String, digits: &mut String) {
    if digits.is_empty() {
        return;
    }
    key.extend(std::iter::repeat_n(
        '0',
        DIGIT_WIDTH.saturating_sub(digits.len()),
    ));
    key.push_str(digits);
    digits.clear();
}

#[cfg(test)]
mod tests {
    use super::natural_key;

    #[test]
    fn numbers_compare_by_value() {
        assert!(natural_key("Vol.2") < natural_key("Vol.10"));
        assert!(natural_key("第9话") < natural_key("第120话"));
    }

    #[test]
    fn case_is_ignored() {
        assert_eq!(natural_key("Summer"), natural_key("SUMMER"));
    }

    #[test]
    fn leading_zeros_do_not_change_the_value() {
        assert_eq!(natural_key("ch 007"), natural_key("ch 7"));
    }
}
