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

use crate::model::SystemSnapshot;

/// Sanitises every display string in a snapshot in place.
///
/// Call this at every untrusted data boundary: the native probe (OS-provided
/// strings) and the browser shell (agent-provided JSON). Fields that are not
/// rendered as text (numbers, timestamps, keys) are left untouched.
pub fn sanitize_snapshot(snapshot: &mut SystemSnapshot) {
    let text = |value: &mut Option<String>| {
        if let Some(inner) = value {
            *inner = sanitize_display(inner, MAX_NAME_CHARS);
        }
    };
    text(&mut snapshot.hostname);
    text(&mut snapshot.os_name);
    text(&mut snapshot.kernel_version);
    for process in &mut snapshot.processes {
        process.name = sanitize_display(&process.name, MAX_NAME_CHARS);
        process.command = sanitize_display(&process.command, MAX_COMMAND_CHARS);
        text(&mut process.user);
    }
    for interface in &mut snapshot.network.interfaces {
        interface.name = sanitize_display(&interface.name, MAX_NAME_CHARS);
    }
    for disk in &mut snapshot.disks {
        // A mount point is a path, not a short label: it can legitimately be
        // far longer than a process name. Use the same limit as the native
        // probe (see `tm-platform`'s `disk_snapshots`) so a ~100-character
        // path is not cut on one sanitisation path but kept on the other.
        disk.mount_point = sanitize_display(&disk.mount_point, MAX_COMMAND_CHARS);
    }
    for component in &mut snapshot.components {
        component.label = sanitize_display(&component.label, MAX_NAME_CHARS);
    }
}

/// Removes ANSI/terminal escape sequences (including the whole payload of OSC
/// and other control strings), control characters, Unicode bidi control
/// characters and zero-width / invisible format characters, collapses whitespace
/// and truncates to `max_chars` characters (not bytes).
///
/// Truncation happens on a character boundary and appends `…`, so CJK process
/// names are never cut in the middle of a code point. Whitespace past the cap
/// (including trailing whitespace) is trimmed and does not add an ellipsis when
/// no visible character was lost.
pub fn sanitize_display(input: &str, max_chars: usize) -> String {
    let stripped = strip_ansi(input);
    let mut out = String::with_capacity(stripped.len().min(max_chars.saturating_mul(4)));
    let mut previous_was_space = false;
    let mut kept = 0usize;
    let mut truncated = false;

    for ch in stripped.chars() {
        // U+2028 (LINE SEPARATOR) / U+2029 (PARAGRAPH SEPARATOR) are Zl/Zp,
        // not Cc, so `is_control()` misses them even though renderers treat
        // them as line breaks. Collapse only these two explicitly: using the
        // whole `is_whitespace()` set would also fold characters our Cf policy
        // already handles (e.g. NBSP) and change established behaviour.
        let ch = if ch.is_control()
            || ch == '\u{2028}'
            || ch == '\u{2029}'
            || is_bidi_control(ch)
            || is_invisible_format(ch)
            || is_format_char(ch)
        {
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
            // Whitespace beyond the cap is dropped by the final `trim()`
            // anyway, so it is not lost content. Only a visible character
            // that cannot be kept is a real truncation (L6-BUGS): an input of
            // exactly `max_chars` visible characters plus trailing spaces must
            // not gain a spurious `…`. Keep scanning past the spaces in case a
            // visible character follows them.
            if ch != ' ' {
                truncated = true;
                break;
            }
            continue;
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

/// Returns `true` for invisible format characters that can be used to spoof
/// process names (zero-width joiners/non-joiners, word joiners, and replacement
/// characters).
///
/// These are valid characters in some scripts, but replacing them with a space
/// is safe here: process names are shown for identification and always paired
/// with the numeric PID, so dropping a zero-width modifier cannot change the
/// meaning of a legitimate name in a way that matters for the UI. Combining
/// marks (e.g. U+0300..U+036F) are intentionally kept so accented names render
/// correctly.
///
/// This explicitly covers ZWJ (U+200D) and ZWNJ (U+200C) as well (R1S-DATA-04).
/// They can form legitimate emoji/Indic ligatures, but *deleting* them would
/// silently concatenate adjacent tokens (`cmd\u{200d}exe` -> `cmdexe`), which is
/// a spoofing risk for an identification label. Replacing them with a visible
/// space keeps the gap, stays consistent with every other Cf character, and
/// preserves the Cf-rejection contract. Ligature fidelity loses to predictability.
fn is_invisible_format(ch: char) -> bool {
    matches!(
        ch,
        '\u{00ad}' // SOFT HYPHEN
            | '\u{034f}' // COMBINING GRAPHEME JOINER
            | '\u{061c}' // ARABIC LETTER MARK (also bidi)
            | '\u{115f}'..='\u{1160}' // HANGUL CHOSEONG/KYEOONG FILLER
            | '\u{17b4}'..='\u{17b5}' // KHMER VOWEL INHERENT AQ/AA
            | '\u{180b}'..='\u{180f}' // MONGOLIAN FREE VARIATION SELECTORS
            | '\u{200b}'..='\u{200f}' // ZWSP, ZWNJ, ZWJ, LRM, RLM
            | '\u{202a}'..='\u{202e}' // bidi embedding controls
            | '\u{2060}'..='\u{206f}' // WORD JOINER .. NOMINAL DIGIT SHAPES
            | '\u{3164}' // HANGUL FILLER
            | '\u{fe00}'..='\u{fe0f}' // VARIATION SELECTOR-1..16
            // Supplementary-plane VS17..256. Process names are always paired
            // with a numeric PID, so dropping these invisible modifiers (as we
            // already do for VS1..16) cannot change their meaning (R26 audit).
            | '\u{e0100}'..='\u{e01ef}' // VARIATION SELECTOR-17..256
            | '\u{feff}' // ZERO WIDTH NO-BREAK SPACE (BOM)
            | '\u{ffa0}' // HALFWIDTH HANGUL FILLER
    )
}

/// Returns `true` for every Unicode `General_Category=Cf` (Format) character.
///
/// The ranges below are the complete Cf set of Unicode 16.0. Unlike
/// [`is_bidi_control`] and [`is_invisible_format`] this is a classification
/// table rather than a blacklist of known spoofing characters: an unseen Cf
/// code point is invisible or reorders text, so it must never reach the
/// renderer. Naming the Unicode version here also removes the implicit
/// version dependence (R1S-DATA-01 / R1S-DATA-11).
fn is_format_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{00ad}' // SOFT HYPHEN
            | '\u{0600}'..='\u{0605}' // ARABIC NUMBER SIGN .. ARABIC SIGN SAMVAT
            | '\u{061c}' // ARABIC LETTER MARK
            | '\u{06dd}' // ARABIC END OF AYAH
            | '\u{070f}' // SYRIAC ABBREVIATION MARK
            | '\u{0890}'..='\u{0891}' // ARABIC POUND / PIASTRE MARK ABOVE
            | '\u{08e2}' // ARABIC DISPUTED END OF AYAH
            | '\u{180e}' // MONGOLIAN VOWEL SEPARATOR
            | '\u{200b}'..='\u{200f}' // ZWSP, ZWNJ, ZWJ, LRM, RLM
            | '\u{202a}'..='\u{202e}' // bidi embedding controls
            | '\u{2060}'..='\u{2064}' // WORD JOINER .. INVISIBLE PLUS
            | '\u{2066}'..='\u{206f}' // bidi isolates .. NOMINAL DIGIT SHAPES
            | '\u{feff}' // ZERO WIDTH NO-BREAK SPACE (BOM)
            | '\u{fff9}'..='\u{fffb}' // INTERLINEAR ANNOTATION ANCHOR/SEPARATOR/TERMINATOR
            | '\u{110bd}' // KAITHI NUMBER SIGN
            | '\u{110cd}' // KAITHI NUMBER SIGN ABOVE
            | '\u{13430}'..='\u{1343f}' // EGYPTIAN HIEROGLYPH format controls
            | '\u{1bca0}'..='\u{1bca3}' // SHORTHAND FORMAT controls
            | '\u{1d173}'..='\u{1d17a}' // MUSICAL control characters
            | '\u{e0001}' // LANGUAGE TAG
            | '\u{e0020}'..='\u{e007f}' // TAG SPACE .. CANCEL TAG
    )
}

/// Removes CSI, string-carrying control sequences and two-character escapes.
///
/// The parser is deliberately small: it only needs to be correct enough that no
/// escape sequence or control-string payload can reach the renderer, not to
/// interpret terminal output.
///
/// OSC (`ESC ]`), DCS (`ESC P`), SOS (`ESC X`), PM (`ESC ^`) and APC (`ESC _`)
/// open a control string whose body is freely printable. Leaving that body in
/// place would let a process name inject a window title, a hyperlink or arbitrary
/// text into a terminal-like widget (R1S-DATA-03). The entire string is therefore
/// consumed up to its terminator — ST (`ESC \`) or BEL (the common OSC
/// terminator) — and a string with no terminator is drained to the end of input.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            // CSI: ESC [ ... final byte in 0x40..=0x7e
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            // ST-terminated control strings: drop the whole payload.
            Some(']' | 'P' | 'X' | '^' | '_') => {
                chars.next(); // the introducer byte
                consume_control_string(&mut chars);
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

/// Drains the payload of an ST-terminated control string.
///
/// Stops at ST (`ESC \`) or BEL. An unterminated string is consumed to the end
/// of the input so that no printable payload survives into the renderer.
fn consume_control_string(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(ch) = chars.next() {
        if ch == '\u{7}' {
            return;
        }
        if ch == '\u{1b}' && chars.peek() == Some(&'\\') {
            chars.next(); // consume the '\\' completing ST
            return;
        }
    }
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
        // R1S-DATA-03: the OSC title payload is dropped; only trailing text stays.
        assert_eq!(sanitize_display("\u{1b}]0;title\u{7}after", 64), "after");
    }

    #[test]
    fn osc_and_dcs_payloads_are_removed() {
        // R1S-DATA-03: OSC with BEL and with ST terminators, and DCS with ST.
        assert_eq!(
            sanitize_display("pre\u{1b}]0;evil-title\u{7}post", 64),
            "prepost"
        );
        assert_eq!(
            sanitize_display("pre\u{1b}]8;;https://evil.example\u{1b}\\post", 64),
            "prepost"
        );
        assert_eq!(
            sanitize_display("pre\u{1b}P+qd0payload\u{1b}\\post", 64),
            "prepost"
        );
        // SOS / PM / APC share the same ST-terminated payload shape.
        assert_eq!(sanitize_display("pre\u{1b}Xsos\u{1b}\\post", 64), "prepost");
        assert_eq!(sanitize_display("pre\u{1b}^pm\u{1b}\\post", 64), "prepost");
        assert_eq!(sanitize_display("pre\u{1b}_apc\u{1b}\\post", 64), "prepost");
        // Unterminated control strings are drained, never echoed.
        assert_eq!(sanitize_display("pre\u{1b}]0;no-terminator", 64), "pre");
        assert_eq!(sanitize_display("pre\u{1b}Punterminated", 64), "pre");
        // An OSC payload containing a CSI must not leak the CSI either.
        assert_eq!(
            sanitize_display("pre\u{1b}]0;\u{1b}[31mred\u{7}post", 64),
            "prepost"
        );
    }

    #[test]
    fn zwj_and_zwnj_are_neutralised_with_a_visible_gap() {
        // R1S-DATA-04 decision: keep "replace Cf with a space" for ZWJ/ZWNJ.
        // Deleting them would glue tokens together (`cmd<ZWJ>exe` -> `cmdexe`),
        // a spoofing risk; ligature fidelity loses to a predictable gap.
        assert_eq!(sanitize_display("cmd\u{200d}exe", 64), "cmd exe");
        assert_eq!(sanitize_display("cmd\u{200c}exe", 64), "cmd exe");
        // Visible glyphs survive; only the invisible joiner becomes a space.
        assert_eq!(
            sanitize_display("\u{1f468}\u{200d}\u{1f469}", 64),
            "\u{1f468} \u{1f469}"
        );
        assert_eq!(sanitize_display("क\u{200c}ष", 64), "क ष");
    }

    #[test]
    fn cf_rejection_survives_control_string_stripping() {
        // R1S-DATA-03/04: removing a control string must not disable the Cf
        // rejection applied to the surrounding text.
        let input = "\u{1b}]0;title\u{7}a\u{200d}\u{200c}\u{202e}b \u{1b}Pdcs\u{1b}\\c";
        let output = sanitize_display(input, 64);
        assert_eq!(output, "a b c");
        assert!(output.chars().all(|ch| !is_format_char(ch)));
    }

    #[test]
    fn bidi_control_characters_are_removed() {
        assert_eq!(sanitize_display("a\u{202e}b\u{2066}c", 64), "a b c");
        assert_eq!(sanitize_display("\u{200f}file", 64), "file");
    }

    #[test]
    fn other_invisible_format_characters_are_removed() {
        // These format characters are not bidi controls and not caught by the
        // common ZWSP/VS ranges, but can still spoof or pad a process name.
        for ch in [
            '\u{034f}',  // COMBINING GRAPHEME JOINER
            '\u{061c}',  // ARABIC LETTER MARK
            '\u{115f}',  // HANGUL CHOSEONG FILLER
            '\u{1160}',  // HANGUL JUNGSEONG FILLER
            '\u{17b4}',  // KHMER VOWEL INHERENT AQ
            '\u{180b}',  // MONGOLIAN FREE VARIATION SELECTOR ONE
            '\u{3164}',  // HANGUL FILLER
            '\u{ffa0}',  // HALFWIDTH HANGUL FILLER
            '\u{e0100}', // VARIATION SELECTOR-17 (supplementary plane)
            '\u{e01ef}', // VARIATION SELECTOR-256
        ] {
            assert!(
                is_invisible_format(ch),
                "U+{:04X} must be invisible",
                ch as u32
            );
        }
        assert_eq!(sanitize_display("a\u{e0100}b", 64), "a b");
        assert_eq!(sanitize_display("a\u{e01ef}b", 64), "a b");
    }

    #[test]
    fn all_bidi_controls_are_neutralised() {
        // Every bidi control in `is_bidi_control`'s ranges must be replaced by
        // a space; a missed one could reorder a process name in the table.
        let controls = [
            '\u{202a}', // LRE
            '\u{202b}', // RLE
            '\u{202c}', // PDF
            '\u{202d}', // LRO
            '\u{202e}', // RLO
            '\u{2066}', // LRI
            '\u{2067}', // RLI
            '\u{2068}', // FSI
            '\u{2069}', // PDI
        ];
        for ch in controls {
            assert!(
                is_bidi_control(ch),
                "U+{:04X} must be a bidi control",
                ch as u32
            );
            assert_eq!(sanitize_display(&format!("a{ch}b"), 64), "a b");
        }
    }

    #[test]
    fn zero_width_and_invisible_characters_are_removed() {
        // ZWSP / ZWNJ / ZWJ / word joiner / BOM / variation selector.
        let spoofed = "pay\u{200b}pal\u{200d}\u{fe0f}\u{2060}com";
        assert_eq!(sanitize_display(spoofed, 64), "pay pal com");
        assert_eq!(sanitize_display("\u{feff}visible", 64), "visible");
        assert_eq!(sanitize_display("a\u{00ad}b", 64), "a b");
    }

    #[test]
    fn legitimate_unicode_text_is_preserved() {
        // Combining marks and ordinary CJK remain intact.
        assert_eq!(
            sanitize_display("cafe\u{0301}日本語", 64),
            "cafe\u{0301}日本語"
        );
        assert_eq!(sanitize_display("PowerShell", 64), "PowerShell");
    }

    #[test]
    fn whitespace_is_collapsed_and_trimmed() {
        assert_eq!(sanitize_display("  a    b  ", 64), "a b");
    }

    #[test]
    fn line_and_paragraph_separators_are_folded_to_spaces() {
        // U+2028 (Zl) / U+2029 (Zp) are not `char::is_control()`, but renderers
        // treat them as line breaks. A wire-derived name must not keep them.
        let output = sanitize_display("evil\u{2028}name\u{2029}with\u{2028}breaks", 64);
        assert_eq!(output, "evil name with breaks");
        assert!(!output.contains('\u{2028}'), "U+2028 survived: {output:?}");
        assert!(!output.contains('\u{2029}'), "U+2029 survived: {output:?}");

        // Separators collapse together with ordinary whitespace and are
        // trimmed at the edges, exactly like the spaces they become.
        assert_eq!(
            sanitize_display("  a\u{2028}\u{2029}  b\u{2029}  ", 64),
            "a b"
        );
    }

    #[test]
    fn truncation_respects_character_boundaries() {
        assert_eq!(sanitize_display("あいうえお", 3), "あいう…");
        assert_eq!(sanitize_display("あいうえお", 10), "あいうえお");
    }

    #[test]
    fn truncation_at_zero_and_one_characters() {
        // A zero-width cap must still produce a finite, non-panicking label.
        // The ellipsis itself is not counted toward `max_chars`.
        assert_eq!(sanitize_display("あいう", 0), "…");
        assert_eq!(sanitize_display("あいう", 1), "あ…");
        assert_eq!(sanitize_display("abc", 0), "…");
        assert_eq!(sanitize_display("abc", 1), "a…");
    }

    #[test]
    fn trailing_whitespace_at_the_limit_does_not_add_an_ellipsis() {
        // L6-BUGS: exactly `max_chars` visible characters followed by
        // whitespace lost no visible content, so no `…` may be appended.
        assert_eq!(sanitize_display("abc  ", 3), "abc");
        assert_eq!(sanitize_display("abc\n\t ", 3), "abc");
        assert_eq!(sanitize_display("あいう ", 3), "あいう");

        let exact = "a".repeat(MAX_NAME_CHARS);
        assert_eq!(
            sanitize_display(&format!("{exact}   "), MAX_NAME_CHARS),
            exact
        );

        // A visible character beyond the cap still truncates.
        assert_eq!(sanitize_display("abc  d", 3), "abc…");
        assert_eq!(sanitize_display("あいう え", 3), "あいう…");
    }

    #[test]
    fn mount_point_keeps_command_length_paths() {
        // L6-BUGS: the snapshot sanitiser and the native probe must use the
        // same limit for mount points. A ~100-character path exceeds
        // MAX_NAME_CHARS but fits MAX_COMMAND_CHARS, so it must survive.
        use crate::demo::demo_snapshot;

        let mut snapshot = demo_snapshot(0);
        let path = format!("/mnt/{}", "data".repeat(25));
        snapshot.disks[0].mount_point = path.clone();
        assert!(path.chars().count() > MAX_NAME_CHARS);
        assert!(path.chars().count() <= MAX_COMMAND_CHARS);

        sanitize_snapshot(&mut snapshot);

        assert_eq!(snapshot.disks[0].mount_point, path);
        assert!(!snapshot.disks[0].mount_point.ends_with('…'));
    }

    #[test]
    fn sanitize_snapshot_covers_every_display_string() {
        use crate::demo::demo_snapshot;

        let mut snapshot = demo_snapshot(0);
        let dirty = "e\u{1b}[31mvil\u{202e}\nname";
        snapshot.hostname = Some(dirty.to_string());
        snapshot.os_name = Some(dirty.to_string());
        snapshot.kernel_version = Some(dirty.to_string());
        snapshot.processes[0].name = dirty.to_string();
        snapshot.processes[0].command = dirty.to_string();
        snapshot.processes[0].user = Some(dirty.to_string());
        snapshot.network.interfaces[0].name = dirty.to_string();
        snapshot.disks[0].mount_point = dirty.to_string();
        snapshot.components[0].label = dirty.to_string();

        sanitize_snapshot(&mut snapshot);
        for value in [
            snapshot.hostname.as_deref().unwrap(),
            snapshot.os_name.as_deref().unwrap(),
            snapshot.kernel_version.as_deref().unwrap(),
            &snapshot.processes[0].name,
            &snapshot.processes[0].command,
            snapshot.processes[0].user.as_deref().unwrap(),
            &snapshot.network.interfaces[0].name,
            &snapshot.disks[0].mount_point,
            &snapshot.components[0].label,
        ] {
            assert!(!value.contains('\u{1b}'), "ANSI survived: {value:?}");
            assert!(!value.contains('\u{202e}'), "bidi survived: {value:?}");
            assert!(!value.contains('\n'), "newline survived: {value:?}");
        }
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
            input.push_str("\u{200b}\u{feff}\u{2060}");

            let output = sanitize_display(&input, 32);
            assert!(output.chars().count() <= 33, "bounded length");
            assert!(
                !output.chars().any(|ch| {
                    ch.is_control()
                        || is_bidi_control(ch)
                        || is_invisible_format(ch)
                        || is_format_char(ch)
                }),
                "no control/bidi/format characters may reach the renderer"
            );
        }
    }

    #[test]
    fn every_unicode_format_character_is_neutralised() {
        // R1S-DATA-01/11: cover all 21 Cf ranges of Unicode 16.0, including the
        // ones the old blacklist missed (Arabic number signs, interlinear
        // annotations, Kaithi, Egyptian hieroglyphs, shorthand, musical and tag
        // characters).
        let controls = [
            '\u{00ad}',
            '\u{0600}',
            '\u{0605}',
            '\u{061c}',
            '\u{06dd}',
            '\u{070f}',
            '\u{0890}',
            '\u{0891}',
            '\u{08e2}',
            '\u{180e}',
            '\u{200b}',
            '\u{200c}',
            '\u{200d}',
            '\u{200e}',
            '\u{200f}',
            '\u{202a}',
            '\u{202e}',
            '\u{2060}',
            '\u{2064}',
            '\u{2066}',
            '\u{2069}',
            '\u{206f}',
            '\u{feff}',
            '\u{fff9}',
            '\u{fffa}',
            '\u{fffb}',
            '\u{110bd}',
            '\u{110cd}',
            '\u{13430}',
            '\u{1343f}',
            '\u{1bca0}',
            '\u{1bca3}',
            '\u{1d173}',
            '\u{1d17a}',
            '\u{e0001}',
            '\u{e0020}',
            '\u{e007f}',
        ];
        for ch in controls {
            assert!(is_format_char(ch), "U+{:04X} must be Cf", ch as u32);
            assert_eq!(
                sanitize_display(&format!("a{ch}b"), 64),
                "a b",
                "U+{:04X} reached the renderer",
                ch as u32
            );
        }
        // Supplementary-plane VS17..256 stay covered (R1S-DATA-02).
        assert_eq!(sanitize_display("a\u{e0100}b", 64), "a b");
        assert_eq!(sanitize_display("a\u{e01ef}b", 64), "a b");
        // Ordinary text and combining marks are still accepted.
        assert!(!is_format_char('あ'));
        assert!(!is_format_char('e'));
        assert!(!is_format_char('\u{0301}'));
    }

    #[test]
    fn huge_command_is_bounded() {
        let input = "x".repeat(10_000);
        let output = sanitize_display(&input, MAX_COMMAND_CHARS);
        assert!(output.chars().count() <= MAX_COMMAND_CHARS + 1);
    }
}
