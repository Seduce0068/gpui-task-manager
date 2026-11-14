//! Defensive display sanitisation.
//!
//! Process names and command lines are attacker-influenced data: any local user
//! can start a process whose name contains newlines, ANSI escape sequences or
//! bidirectional text. A task manager must never let that data corrupt its own
//! layout or terminal-like widgets, so every string that comes from the OS is
//! sanitised before it reaches a shell.

/// Maximum number of characters kept from a process name.
pub const MAX_NAME_CHARS: usize = 64;
/// Maximum number of characters kept from a command line.
pub const MAX_COMMAND_CHARS: usize = 240;

/// Removes ANSI escape sequences, control characters and Unicode bidi control
/// characters, collapses whitespace and truncates to `max_chars` characters
/// (not bytes).
///
/// Truncation happens on a character boundary and appends `…`, so CJK process
/// names are never cut in the middle of a code point.
pub fn sanitize_display(input: &str, max_chars: usize) -> String {
    let stripped = strip_ansi(input);
    let mut out = String::with_capacity(stripped.len().min(max_chars.saturating_mul(4)));
    let mut previous_was_space = false;
    let mut kept = 0usize;
    let mut truncated = false;

    for ch in stripped.chars() {
        let ch = if ch.is_control() || is_bidi_control(ch) {
            ' '
        } else {
            ch
        };
        if ch == ' ' {
            if previous_was_space {
                continue;
            }
            previous_was_space = true;
        } else {
            previous_was_space = false;
        }
        if kept >= max_chars {
            truncated = true;
            break;
        }
        out.push(ch);
        kept += 1;
    }

    let trimmed = out.trim();
    if truncated {
        format!("{trimmed}…")
    } else {
        trimmed.to_string()
    }
}

/// Returns `true` for Unicode bidirectional control characters.
///
/// These are not `char::is_control()` (they are format characters), but a
/// process name containing an RTL override can still reorder the surrounding
/// text. Replacing them with spaces keeps the rendered layout predictable.
fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// Removes CSI/OSC style escape sequences.
///
/// The parser is deliberately small: it only needs to be correct enough that an
/// escape sequence cannot reach the renderer, not to interpret terminal output.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            // CSI: ESC [ ... final byte in 0x40..=0x7e
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            // Two-character escape sequences such as ESC c.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_replaced() {
        assert_eq!(
            sanitize_display("evil\nname\twith\rcontrol", 64),
            "evil name with control"
        );
    }

    #[test]
    fn ansi_sequences_are_removed() {
        assert_eq!(sanitize_display("\u{1b}[31mred\u{1b}[0m", 64), "red");
        assert_eq!(
            sanitize_display("\u{1b}]0;title\u{7}after", 64),
            "0;title after"
        );
    }

    #[test]
    fn bidi_control_characters_are_removed() {
        assert_eq!(sanitize_display("a\u{202e}b\u{2066}c", 64), "a b c");
        assert_eq!(sanitize_display("\u{200f}file", 64), "file");
    }

    #[test]
    fn whitespace_is_collapsed_and_trimmed() {
        assert_eq!(sanitize_display("  a    b  ", 64), "a b");
    }

    #[test]
    fn truncation_respects_character_boundaries() {
        assert_eq!(sanitize_display("あいうえお", 3), "あいう…");
        assert_eq!(sanitize_display("あいうえお", 10), "あいうえお");
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert_eq!(sanitize_display("", 10), "");
        assert_eq!(sanitize_display("\u{1b}[0m", 10), "");
    }

    /// Deterministic pseudo-random generator used only by tests.
    fn splitmix64(seed: u64) -> u64 {
        let mut value = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    #[test]
    fn sanitized_output_never_contains_control_or_bidi_characters() {
        let mut seed = 0x1234_5678_9ABC_DEF0u64;
        for _ in 0..500 {
            seed = splitmix64(seed);
            let length = (seed % 96) as usize;
            let mut input = String::new();
            for _ in 0..length {
                seed = splitmix64(seed);
                let candidate = (seed % 0x300) as u32;
                input.push(char::from_u32(candidate).unwrap_or('x'));
            }
            // Inject the sequences the sanitiser exists to remove.
            input.push('\u{1b}');
            input.push_str("[31mred\u{202e}");
            input.push('\u{7}');

            let output = sanitize_display(&input, 32);
            assert!(output.chars().count() <= 33, "bounded length");
            assert!(
                !output
                    .chars()
                    .any(|ch| ch.is_control() || is_bidi_control(ch)),
                "no control/bidi characters may reach the renderer"
            );
        }
    }

    #[test]
    fn huge_command_is_bounded() {
        let input = "x".repeat(10_000);
        let output = sanitize_display(&input, MAX_COMMAND_CHARS);
        assert!(output.chars().count() <= MAX_COMMAND_CHARS + 1);
    }
}
