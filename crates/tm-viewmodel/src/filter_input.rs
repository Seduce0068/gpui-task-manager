//! 絞り込み入力フィールドの状態機械（純粋ロジック）。
//!
//! GPUI の `EntityInputHandler` はテキスト・選択範囲・IME の未確定文字列
//! （marked range）を **UTF-16 コードユニット**のオフセットでやり取りする。
//! このモジュールはその状態だけを保持し、DOM にも GPUI にも依存しないため、
//! ホスト（`cargo test`）で単体テストできる。
//!
//! フィルタへ反映するのは **確定済み文字列** だけ。IME 変換中の文字列は
//! 表示（下線・色分け）にだけ使い、[`crate::Dashboard`] の絞り込みには
//! 渡さない。変換中の文字で一覧がちらつかないようにするための境界である。
//!
//! オフセットはすべて UTF-16 コードユニット（GPUI / ブラウザの IME API と同じ）。
//! `String` のバイト境界へ変換するときは、サロゲートペアの途中でも安全なように
//! 「その UTF-16 位置以下で最大の文字境界」へ丸める。

use std::ops::Range;

/// 編集操作の結果。呼び出し側はこれを見てフィルタへ反映するか決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOutcome {
    /// テキストも変換中範囲も変わらなかった（何もしない）。
    Unchanged,
    /// テキストは変わったが IME 変換中。確定文字列は変わらないのでフィルタは更新しない。
    Composing,
    /// 確定済み文字列が変わった。フィルタへ反映する。
    Committed,
}

/// 絞り込み欄の編集状態。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterInput {
    /// 表示テキスト（IME 変換中の文字列も含む）。
    text: String,
    /// 選択範囲（UTF-16、常に `0..=utf16_len()` にクランプ済み）。
    selection: Range<usize>,
    /// IME 変換中の範囲（UTF-16）。`None` なら変換中ではない。
    marked: Option<Range<usize>>,
}

impl FilterInput {
    /// 空の入力状態。
    pub fn new() -> Self {
        Self::default()
    }

    /// 表示テキスト（変換中文字列を含む）。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 現在の選択範囲（UTF-16）。
    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    /// IME 変換中の範囲（UTF-16）。
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// IME 変換中か。
    pub fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    /// テキスト長（UTF-16 コードユニット）。
    pub fn utf16_len(&self) -> usize {
        self.text.encode_utf16().count()
    }

    /// 表示テキストが空か。
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// 表示用の 3 分割 `(変換前, 変換中, 変換後)`。変換中が無ければ中央は空文字。
    pub fn display_parts(&self) -> (&str, &str, &str) {
        match &self.marked {
            Some(range) => {
                let start = floor_byte_index(&self.text, range.start);
                let end = floor_byte_index(&self.text, range.end);
                let end = end.max(start);
                (
                    &self.text[..start],
                    &self.text[start..end],
                    &self.text[end..],
                )
            }
            None => (&self.text, "", ""),
        }
    }

    /// フィルタへ反映する確定済み文字列（変換中範囲を取り除いたもの）。
    pub fn committed_text(&self) -> String {
        match &self.marked {
            Some(range) => {
                let start = floor_byte_index(&self.text, range.start);
                let end = floor_byte_index(&self.text, range.end).max(start);
                let mut committed = String::with_capacity(self.text.len());
                committed.push_str(&self.text[..start]);
                committed.push_str(&self.text[end..]);
                committed
            }
            None => self.text.clone(),
        }
    }

    /// `range` の範囲を `text` で置き換える（IME の確定・貼り付け・キー入力）。
    ///
    /// `range` が `None` のときは、GPUI の慣例どおり「変換中範囲 → 選択範囲」の順で
    /// 編集対象を決める。置換後は変換中状態を解除する。
    pub fn replace_text(&mut self, range: Option<Range<usize>>, text: &str) -> EditOutcome {
        let target = self.resolve_range(range);
        self.apply(target, text, false, None)
    }

    /// `range` の範囲を `text` で置き換え、その範囲を IME 変換中（marked）にする。
    ///
    /// ブラウザの `compositionupdate` は「変換中の文字列全体」を渡してくるので、
    /// 既存の marked range を新しい文字列で置き換える（重複させない）。
    pub fn replace_and_mark(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected_range: Option<Range<usize>>,
    ) -> EditOutcome {
        let target = self.resolve_range(range);
        self.apply(target, text, true, selected_range)
    }

    /// 変換中状態を解除する（`compositionend` 後の後始末）。確定文字列は変えない。
    pub fn unmark(&mut self) -> EditOutcome {
        if self.marked.take().is_some() {
            EditOutcome::Composing
        } else {
            EditOutcome::Unchanged
        }
    }

    /// 入力を空にする（Esc / クリアボタン）。
    pub fn clear(&mut self) -> EditOutcome {
        let changed = !self.text.is_empty() || self.marked.is_some();
        self.text.clear();
        self.selection = 0..0;
        self.marked = None;
        if changed {
            EditOutcome::Committed
        } else {
            EditOutcome::Unchanged
        }
    }

    /// 選択範囲、またはキャレット直前の 1 文字を削除する（Backspace）。
    ///
    /// Web ではブラウザの既定動作 → `input` 差分取り込みで同じ結果になるが、
    /// ネイティブではキーイベントをアプリ側で処理する必要があるため共有する。
    pub fn backspace(&mut self) -> EditOutcome {
        let selection = self.selection.clone();
        if selection.start != selection.end {
            return self.replace_text(Some(selection), "");
        }
        let end = selection.start;
        if end == 0 {
            return EditOutcome::Unchanged;
        }
        let start = previous_char_start(&self.text, end);
        self.replace_text(Some(start..end), "")
    }

    /// 選択範囲を設定する（ブラウザの `selectionchange` 取り込み）。
    pub fn set_selection(&mut self, range: Range<usize>) {
        let len = self.utf16_len();
        let start = range.start.min(len);
        let end = range.end.min(len).max(start);
        self.selection = start..end;
    }

    /// UTF-16 範囲に対応する文字列を返す。丸めた実際の範囲も返す。
    pub fn text_for_range(&self, range: Range<usize>) -> (String, Range<usize>) {
        let len = self.utf16_len();
        let start = range.start.min(len);
        let end = range.end.min(len).max(start);
        let byte_start = floor_byte_index(&self.text, start);
        let byte_end = floor_byte_index(&self.text, end).max(byte_start);
        let actual = utf16_index(&self.text, byte_start)..utf16_index(&self.text, byte_end);
        (self.text[byte_start..byte_end].to_string(), actual)
    }

    // ------------------------------------------------------------ internal

    /// 編集対象の範囲を決める: 明示 range → 変換中範囲 → 選択範囲。
    fn resolve_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        let len = self.utf16_len();
        let raw = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        let start = raw.start.min(len);
        let end = raw.end.min(len).max(start);
        start..end
    }

    fn apply(
        &mut self,
        target: Range<usize>,
        new_text: &str,
        mark: bool,
        selected_range: Option<Range<usize>>,
    ) -> EditOutcome {
        let old_text = self.text.clone();
        let old_marked = self.marked.clone();

        let byte_start = floor_byte_index(&self.text, target.start);
        let byte_end = floor_byte_index(&self.text, target.end).max(byte_start);
        self.text.replace_range(byte_start..byte_end, new_text);

        let inserted_start = target.start;
        let inserted_end = inserted_start + new_text.encode_utf16().count();
        let inserted = inserted_start..inserted_end;

        match selected_range {
            // IME が変換中の選択位置を指定してきた場合は、挿入範囲からの相対として解釈する。
            Some(relative) if mark => {
                let start = inserted_start + relative.start.min(inserted_end - inserted_start);
                let end = inserted_start + relative.end.min(inserted_end - inserted_start);
                self.selection = start.min(end)..end.max(start);
            }
            _ => self.selection = inserted_end..inserted_end,
        }

        self.marked = if mark { Some(inserted) } else { None };

        if self.text == old_text && self.marked == old_marked {
            return EditOutcome::Unchanged;
        }
        if self.marked.is_some() {
            EditOutcome::Composing
        } else {
            EditOutcome::Committed
        }
    }
}

/// UTF-16 オフセット以下で最大の `String` バイト境界を返す（サロゲートペアの途中でも安全）。
fn floor_byte_index(text: &str, utf16_index: usize) -> usize {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        let width = character.len_utf16();
        if units + width > utf16_index {
            return byte;
        }
        units += width;
        if units == utf16_index {
            return byte + character.len_utf8();
        }
    }
    text.len()
}

/// バイト境界に対応する UTF-16 オフセットを返す。
fn utf16_index(text: &str, byte_index: usize) -> usize {
    text[..byte_index.min(text.len())]
        .chars()
        .map(char::len_utf16)
        .sum()
}

/// UTF-16 オフセット `utf16_index` の直前にある文字の開始 UTF-16 オフセット。
fn previous_char_start(text: &str, utf16_index: usize) -> usize {
    let byte = floor_byte_index(text, utf16_index);
    match text[..byte].chars().next_back() {
        Some(character) => utf16_index.saturating_sub(character.len_utf16()),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_typing_appends_and_commits() {
        let mut input = FilterInput::new();
        assert_eq!(input.replace_text(None, "a"), EditOutcome::Committed);
        assert_eq!(input.replace_text(None, "bc"), EditOutcome::Committed);
        assert_eq!(input.text(), "abc");
        assert_eq!(input.committed_text(), "abc");
        assert_eq!(input.selection(), 3..3);
        assert!(!input.is_composing());
    }

    #[test]
    fn composition_is_not_committed_until_it_ends() {
        let mut input = FilterInput::new();

        // compositionstart + compositionupdate: 変換中文字列だけが入る。
        assert_eq!(
            input.replace_and_mark(None, "にほんご", None),
            EditOutcome::Composing
        );
        assert_eq!(input.text(), "にほんご");
        assert_eq!(input.marked_range(), Some(0..4));
        assert!(input.is_composing());
        // 変換中はフィルタへ渡す確定文字列が空のまま。
        assert_eq!(input.committed_text(), "");

        // 候補の切り替え（compositionupdate が文字列全体を渡す）。
        assert_eq!(
            input.replace_and_mark(None, "日本語", None),
            EditOutcome::Composing
        );
        assert_eq!(input.text(), "日本語");
        assert_eq!(input.marked_range(), Some(0..3));
        assert_eq!(input.committed_text(), "");

        // compositionend: 確定文字列を反映して変換中を解除する。
        assert_eq!(input.replace_text(None, "日本語"), EditOutcome::Committed);
        assert_eq!(input.text(), "日本語");
        assert_eq!(input.committed_text(), "日本語");
        assert_eq!(input.marked_range(), None);
        assert!(!input.is_composing());
    }

    #[test]
    fn composition_after_existing_text_keeps_committed_prefix() {
        let mut input = FilterInput::new();
        input.replace_text(None, "fire");
        input.replace_and_mark(None, "にほんご", None);
        assert_eq!(input.text(), "fireにほんご");
        assert_eq!(input.committed_text(), "fire");
        input.replace_text(None, "日本語");
        assert_eq!(input.committed_text(), "fire日本語");
    }

    #[test]
    fn composition_in_the_middle_of_text_keeps_suffix() {
        let mut input = FilterInput::new();
        input.replace_text(None, "abcdef");
        input.set_selection(3..3);
        input.replace_and_mark(None, "にほんご", None);
        assert_eq!(input.text(), "abcにほんごdef");
        assert_eq!(input.committed_text(), "abcdef");
        input.replace_text(None, "日本語");
        assert_eq!(input.text(), "abc日本語def");
        assert_eq!(input.committed_text(), "abc日本語def");
    }

    #[test]
    fn unmark_without_commit_keeps_committed_text_unchanged() {
        let mut input = FilterInput::new();
        input.replace_text(None, "abc");
        input.replace_and_mark(None, "にほんご", None);
        // 変換をキャンセルした場合: 表示は変換中文字列のまま、確定は "abc"。
        assert_eq!(input.unmark(), EditOutcome::Composing);
        assert!(!input.is_composing());
        // unmark 自体はテキストを消さない（ブラウザ側が空文字で確定しに来るのを待つ）。
        assert_eq!(input.text(), "abcにほんご");
    }

    #[test]
    fn clear_resets_text_selection_and_marked() {
        let mut input = FilterInput::new();
        input.replace_text(None, "日本語");
        input.replace_and_mark(None, "にほん", None);
        assert_eq!(input.clear(), EditOutcome::Committed);
        assert_eq!(input.text(), "");
        assert_eq!(input.committed_text(), "");
        assert_eq!(input.selection(), 0..0);
        assert_eq!(input.marked_range(), None);
        assert_eq!(input.clear(), EditOutcome::Unchanged);
    }

    #[test]
    fn backspace_is_an_explicit_range_replacement() {
        // ブラウザの input イベント取り込みは「選択位置からの相対範囲」を渡す。
        let mut input = FilterInput::new();
        input.replace_text(None, "日本語abc");
        let len = input.utf16_len();
        assert_eq!(len, 6);
        assert_eq!(input.replace_text(Some(5..6), ""), EditOutcome::Committed);
        assert_eq!(input.text(), "日本語ab");
        assert_eq!(input.selection(), 5..5);
    }

    #[test]
    fn surrogate_pairs_are_measured_in_utf16_units() {
        let mut input = FilterInput::new();
        input.replace_text(None, "🦀"); // U+1F980 は UTF-16 で 2 コードユニット
        assert_eq!(input.utf16_len(), 2);
        assert_eq!(input.text(), "🦀");
        assert_eq!(input.committed_text(), "🦀");

        // サロゲートペアの途中を指す範囲でも panic しない（文字境界へ丸める）。
        let (text, actual) = input.text_for_range(0..1);
        assert_eq!(text, "");
        assert_eq!(actual, 0..0);

        // サロゲートペアを選択して変換を開始する。
        input.set_selection(0..2);
        input.replace_and_mark(None, "かに🦀", None);
        assert_eq!(input.utf16_len(), 4);
        assert_eq!(input.text(), "かに🦀");
        assert_eq!(input.marked_range(), Some(0..4));
        assert_eq!(input.committed_text(), "");
        input.replace_text(None, "蟹🦀");
        assert_eq!(input.committed_text(), "蟹🦀");
        assert_eq!(input.utf16_len(), 3);
    }

    #[test]
    fn text_for_range_uses_utf16_offsets() {
        let mut input = FilterInput::new();
        input.replace_text(None, "日本語abc");
        let (text, actual) = input.text_for_range(1..3);
        assert_eq!(text, "本語");
        assert_eq!(actual, 1..3);

        let (text, actual) = input.text_for_range(0..100);
        assert_eq!(text, "日本語abc");
        assert_eq!(actual, 0..6);
    }

    #[test]
    fn selection_replacement_is_used_when_composition_starts() {
        let mut input = FilterInput::new();
        input.replace_text(None, "abc");
        input.set_selection(0..3);
        input.replace_and_mark(None, "にほんご", None);
        assert_eq!(input.text(), "にほんご");
        assert_eq!(input.marked_range(), Some(0..4));
        input.replace_text(None, "日本語");
        assert_eq!(input.committed_text(), "日本語");
    }

    #[test]
    fn backspace_deletes_the_previous_character() {
        let mut input = FilterInput::new();
        input.replace_text(None, "日本語abc");
        assert_eq!(input.backspace(), EditOutcome::Committed);
        assert_eq!(input.text(), "日本語ab");
        assert_eq!(input.committed_text(), "日本語ab");
        assert_eq!(input.selection(), 5..5);

        // 変換中は表示テキストの末尾 1 文字を消し、変換中状態を解除する
        // （実 IME は変換中の Backspace を自分で処理するため、これは保険）。
        input.replace_and_mark(None, "にほんご", None);
        assert_eq!(input.backspace(), EditOutcome::Committed);
        assert_eq!(input.committed_text(), "日本語abにほん");
        assert!(!input.is_composing());
    }

    #[test]
    fn backspace_replaces_a_selection_and_handles_surrogate_pairs() {
        let mut input = FilterInput::new();
        input.replace_text(None, "abc");
        input.set_selection(1..3);
        assert_eq!(input.backspace(), EditOutcome::Committed);
        assert_eq!(input.text(), "a");

        // 絵文字（UTF-16 で 2 コードユニット）も 1 文字として消える。
        input.clear();
        input.replace_text(None, "🦀");
        assert_eq!(input.backspace(), EditOutcome::Committed);
        assert_eq!(input.text(), "");

        // 先頭での Backspace は何もしない。
        assert_eq!(input.backspace(), EditOutcome::Unchanged);
    }

    #[test]
    fn display_parts_splits_composition() {
        let mut input = FilterInput::new();
        input.replace_text(None, "abc");
        input.replace_and_mark(None, "にほん", None);
        let (before, composing, after) = input.display_parts();
        assert_eq!(before, "abc");
        assert_eq!(composing, "にほん");
        assert_eq!(after, "");
        input.replace_text(None, "日本");
        assert_eq!(input.display_parts(), ("abc日本", "", ""));
    }
}
