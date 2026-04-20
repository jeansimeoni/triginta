// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jean Simeoni

use crate::config::GlyphMode;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Symbols {
    pub(crate) timer: &'static str,
    pub(crate) favorite: &'static str,
    pub(crate) project: &'static str,
    pub(crate) section: &'static str,
    pub(crate) tag: &'static str,
    pub(crate) filter: &'static str,
    pub(crate) tasks: &'static str,
    pub(crate) inbox: &'static str,
    pub(crate) today: &'static str,
    pub(crate) soon: &'static str,
    pub(crate) details: &'static str,
    pub(crate) stats: &'static str,
    pub(crate) sort: &'static str,
    pub(crate) search: &'static str,
    pub(crate) hidden: &'static str,
    pub(crate) visible: &'static str,
    pub(crate) recurring: &'static str,
    pub(crate) collapsed: &'static str,
    pub(crate) expanded: &'static str,
    pub(crate) priority: &'static str,
    pub(crate) todo: &'static str,
    pub(crate) in_progress: &'static str,
    pub(crate) breaking: &'static str,
    pub(crate) done: &'static str,
    pub(crate) voided: &'static str,
    pub(crate) delete: &'static str,
    pub(crate) save: &'static str,
    pub(crate) assign: &'static str,
    pub(crate) confirm: &'static str,
    pub(crate) move_hint: &'static str,
    pub(crate) active_option_marker: &'static str,
    pub(crate) new_item: &'static str,
    pub(crate) bar_full: &'static str,
    pub(crate) bar_empty: &'static str,
    pub(crate) tag_chip_left: &'static str,
    pub(crate) tag_chip_right: &'static str,
    pub(crate) tag_chip_uses_background: bool,
    pub(crate) markdown_quote_prefix: &'static str,
    pub(crate) markdown_heading_bar: &'static str,
    pub(crate) markdown_checkbox_empty: &'static str,
    pub(crate) markdown_checkbox_done: &'static str,
    pub(crate) markdown_bullet: &'static str,
    ascii_mode: bool,
}

impl Symbols {
    pub(crate) fn new(mode: GlyphMode) -> Self {
        match mode {
            GlyphMode::Ascii => Self {
                timer: "*",
                favorite: "*",
                project: "P",
                section: "S",
                tag: "@",
                filter: "f",
                tasks: "#",
                inbox: "I",
                today: "T",
                soon: "S",
                details: ">",
                stats: "%",
                sort: "~",
                search: "/",
                hidden: "x",
                visible: "o",
                recurring: "~",
                collapsed: "+",
                expanded: "-",
                priority: "!",
                todo: ".",
                in_progress: ">",
                breaking: "~",
                done: "x",
                voided: "!",
                delete: "x",
                save: "Enter",
                assign: "Enter",
                confirm: "Enter",
                move_hint: "j/k",
                active_option_marker: "*",
                new_item: "+",
                bar_full: "=",
                bar_empty: "-",
                tag_chip_left: "[",
                tag_chip_right: "]",
                tag_chip_uses_background: false,
                markdown_quote_prefix: ">",
                markdown_heading_bar: "#",
                markdown_checkbox_empty: "[ ]",
                markdown_checkbox_done: "[x]",
                markdown_bullet: "-",
                ascii_mode: true,
            },
            GlyphMode::NerdFonts => Self {
                timer: "󰔛",
                favorite: "󰓎",
                project: "󰉋",
                section: "󰙅",
                tag: "󰓹",
                filter: "󰈲",
                tasks: "",
                inbox: "",
                today: "󰃰",
                soon: "󰸘",
                details: "󰋼",
                stats: "",
                sort: "󰒺",
                search: "󰍉",
                hidden: "󰈉",
                visible: "󰈈",
                recurring: "󰑖",
                collapsed: "▸",
                expanded: "▾",
                priority: "⚑",
                todo: "󰄱",
                in_progress: "󰧞",
                breaking: "󰒲",
                done: "󰄵",
                voided: "󰅖",
                delete: "󰆴",
                save: "󰆓",
                assign: "",
                confirm: "󰄵",
                move_hint: "󰄾",
                active_option_marker: "󰄵",
                new_item: "✚",
                bar_full: "█",
                bar_empty: "░",
                tag_chip_left: "",
                tag_chip_right: "",
                tag_chip_uses_background: true,
                markdown_quote_prefix: "▏",
                markdown_heading_bar: "▌",
                markdown_checkbox_empty: "☐",
                markdown_checkbox_done: "☑",
                markdown_bullet: "•",
                ascii_mode: false,
            },
        }
    }

    pub(crate) fn is_ascii(self) -> bool {
        self.ascii_mode
    }

    pub(crate) fn sort_footer_prefix(self) -> &'static str {
        if self.is_ascii() { "sort" } else { self.sort }
    }

    pub(crate) fn done_filter_prefix(self, hides_completed: bool) -> &'static str {
        if self.is_ascii() {
            "done"
        } else if hides_completed {
            self.hidden
        } else {
            self.visible
        }
    }

    pub(crate) fn active_option_prefix(self) -> String {
        format!("{} ", self.active_option_marker)
    }
}

#[cfg(test)]
mod tests {
    use super::Symbols;
    use crate::config::GlyphMode;

    #[test]
    fn symbols_ascii_mode_uses_expected_delete_and_markdown_fallbacks() {
        let symbols = Symbols::new(GlyphMode::Ascii);
        assert!(symbols.is_ascii());
        assert_eq!(symbols.delete, "x");
        assert_eq!(symbols.markdown_checkbox_empty, "[ ]");
        assert_eq!(symbols.markdown_checkbox_done, "[x]");
        assert_eq!(symbols.markdown_bullet, "-");
    }

    #[test]
    fn symbols_nerd_mode_uses_expected_delete_and_markdown_glyphs() {
        let symbols = Symbols::new(GlyphMode::NerdFonts);
        assert!(!symbols.is_ascii());
        assert_eq!(symbols.delete, "󰆴");
        assert_eq!(symbols.markdown_checkbox_empty, "☐");
        assert_eq!(symbols.markdown_checkbox_done, "☑");
        assert_eq!(symbols.markdown_bullet, "•");
    }
}
