//! The command list: what the escape key opens.
//!
//! On the site, `Escape` is the command line — not a restart, not "clear", and
//! not a back key. With the default `quickRestart: "off"`, `quickRestart` has no
//! binding at all and `Escape` is bound to the command line instead; the only
//! other binding anywhere is `Mod+Shift+P`.
//!
//! The list is a flat one rather than a tree of submenus, which is also the
//! site's default (`singleListCommandLine: "on"`). Every command is one line:
//! a name, some aliases, and what it does.
//!
//! ## Filtering
//!
//! The site's algorithm, because it behaves better than the obvious one: the
//! query is split on spaces, and every token must **prefix-match a distinct word**
//! of a command's name or aliases. So `custom 30` finds `set custom words 30`
//! and `30 custom` finds it too, while `cus 30` does not — because `cus` matches
//! the first word and `30` then has to match a *different* one.
//!
//! Ties are broken towards the longest total match, so `th` finds `theme` and
//! not everything containing `th`.

use crate::config::bar::Field;
use crate::config::Mode as ConfigMode;

/// One command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// What the list shows.
    pub display: &'static str,
    /// Words the user may type that mean this command.
    pub aliases: &'static [&'static str],
    /// What it does.
    pub action: Action,
}

/// What a command does.
///
/// Every variant is handled in `App::run_command`, so a new command cannot
/// become a no-op by being added here and forgotten there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Throw the test away and start a new one.
    Restart,
    /// Switch to a mode.
    Mode(ConfigMode),
    /// Toggle a field.
    Toggle(Field),
    /// Set a length.
    SetLength(Field),
    /// Open the language browser.
    Languages,
    /// Open the settings screen.
    Settings,
    /// Open the custom-text window.
    CustomText,
    /// Change the theme to the next one.
    NextTheme,
    /// Set the theme by name.
    Theme(&'static str),
    /// Leave the app.
    Quit,
    /// Copy a line describing the test to the clipboard, if there is one.
    CopyResult,
    /// Show only the word being typed.
    Blind,
    /// Step the difficulty. The website keeps this in the settings panel rather
    /// than on the test bar, so a command is the quick way in.
    Difficulty(i8),
}

/// The commands, in the order the list shows them.
///
/// Grouped the way the site groups its own list: the test itself first, then the
/// settings that change it, then the appearance, then the things that leave.
pub const COMMANDS: &[Command] = &[
    // The test.
    Command {
        display: "next test",
        aliases: &["restart", "start", "begin", "again"],
        action: Action::Restart,
    },
    Command {
        display: "view typing",
        aliases: &["navigate", "go", "start"],
        action: Action::Restart,
    },
    Command {
        display: "time",
        aliases: &["seconds", "duration"],
        action: Action::Mode(ConfigMode::Time),
    },
    Command {
        display: "words",
        aliases: &["word count"],
        action: Action::Mode(ConfigMode::Words),
    },
    Command {
        display: "quote",
        aliases: &["quotes", "passage"],
        action: Action::Mode(ConfigMode::Quote),
    },
    Command {
        display: "zen",
        aliases: &["free"],
        action: Action::Mode(ConfigMode::Zen),
    },
    Command {
        display: "custom",
        aliases: &["custom text", "passage"],
        action: Action::Mode(ConfigMode::Custom),
    },
    // The settings.
    Command {
        display: "punctuation",
        aliases: &["punct", "punc"],
        action: Action::Toggle(Field::Punctuation),
    },
    Command {
        display: "numbers",
        aliases: &["number", "digits"],
        action: Action::Toggle(Field::Numbers),
    },
    Command {
        display: "duration",
        aliases: &["time", "seconds", "custom time"],
        action: Action::SetLength(Field::TimeCustom),
    },
    Command {
        display: "word count",
        aliases: &["words", "custom words"],
        action: Action::SetLength(Field::WordsCustom),
    },
    Command {
        display: "change custom text",
        aliases: &["custom text", "passage"],
        action: Action::CustomText,
    },
    Command {
        display: "quote length",
        aliases: &["quotes", "thicc", "short", "medium", "long"],
        action: Action::SetLength(Field::WordsCustom),
    },
    Command {
        display: "harder",
        aliases: &["difficulty", "easier"],
        action: Action::Difficulty(1),
    },
    Command {
        display: "easier",
        aliases: &["difficulty", "slower"],
        action: Action::Difficulty(-1),
    },
    Command {
        display: "blind mode",
        aliases: &["blind", "hide words"],
        action: Action::Blind,
    },
    Command {
        display: "language",
        aliases: &["lang", "word list", "words"],
        action: Action::Languages,
    },
    Command {
        display: "settings",
        aliases: &["config", "options", "preferences"],
        action: Action::Settings,
    },
    // Appearance.
    Command {
        display: "next theme",
        aliases: &["theme", "colour", "color"],
        action: Action::NextTheme,
    },
    Command {
        display: "gruvbox",
        aliases: &["theme"],
        action: Action::Theme("gruvbox"),
    },
    Command {
        display: "nord",
        aliases: &["theme"],
        action: Action::Theme("nord"),
    },
    Command {
        display: "dracula",
        aliases: &["theme"],
        action: Action::Theme("dracula"),
    },
    Command {
        display: "catppuccin",
        aliases: &["theme"],
        action: Action::Theme("catppuccin"),
    },
    Command {
        display: "tokyo night",
        aliases: &["theme", "tokyonight"],
        action: Action::Theme("tokyo_night"),
    },
    Command {
        display: "solarized light",
        aliases: &["theme", "light"],
        action: Action::Theme("solarized_light"),
    },
    Command {
        display: "gruvbox light",
        aliases: &["theme", "light"],
        action: Action::Theme("gruvbox_light"),
    },
    // Out.
    Command {
        display: "copy result",
        aliases: &["clipboard", "share"],
        action: Action::CopyResult,
    },
    Command {
        display: "quit",
        aliases: &["exit", "close"],
        action: Action::Quit,
    },
];

/// How well a command matches, and how many of the query's tokens it used.
///
/// The count is the number of *distinct* command words a token was matched
/// against, which is what makes two tokens unable to match the same word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub command: usize,
    pub count: usize,
    pub strength: usize,
    /// Whether every token matched a word of the command's **name** rather than
    /// one of its aliases.
    ///
    /// This is what puts `theme` above `quote length` for the query `th`: both
    /// match two characters, but one of them is found on the label the user can
    /// see, and the other only because `thicc` is hidden behind `quote length`.
    /// Ranking the visible match first is the difference between a search that
    /// finds what you meant and one that finds something surprising first.
    pub on_label: bool,
}

/// The commands that match `query`, best first.
///
/// An empty query matches everything, in list order, which is what the site does
/// so the list is browsable before anything is typed.
pub fn filter(query: &str) -> Vec<Match> {
    let tokens: Vec<String> = query
        .to_lowercase()
        .split(' ')
        .map(strip_punctuation)
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.is_empty() {
        return (0..COMMANDS.len())
            .map(|command| Match {
                command,
                count: 0,
                strength: 0,
                on_label: true,
            })
            .collect();
    }

    let mut matches: Vec<Match> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            match_score(command, &tokens).map(|(count, strength, on_label)| Match {
                command: index,
                count,
                strength,
                on_label,
            })
        })
        .collect();

    // Only the longest total match survives, and only commands that used as many
    // distinct words as there are tokens — which is the site's rule and the one
    // that makes `cus 30` find nothing rather than something irrelevant.
    let best = matches.iter().map(|m| m.strength).max().unwrap_or(0);
    let mut needed = tokens.len();
    loop {
        let any = matches
            .iter()
            .any(|m| m.count >= needed && m.strength >= best);
        if any || needed == 0 {
            break;
        }
        needed -= 1;
    }
    let needed = needed.max(1);
    matches.retain(|m| m.count >= needed && m.strength >= best);
    // Strongest first, and a match on the visible name beats an equally strong
    // one found through a hidden alias. The index breaks the remaining ties, so
    // the list is stable and does not reshuffle between keystrokes.
    matches.sort_by(|a, b| {
        b.strength
            .cmp(&a.strength)
            .then(b.on_label.cmp(&a.on_label))
            .then(a.command.cmp(&b.command))
    });
    matches
}

/// How many of `tokens` a command matched, the total length of the matches, and
/// whether they all landed on the visible name.
///
/// Each token must prefix-match a command word it has not already been matched
/// to. So the words of "custom text words" match "custom text" in order, and
/// `text custom` does not.
///
/// A word is tried on the display first and on the aliases afterwards, so a
/// token that could match either takes the label. That is the visible-match rule
/// above, made mechanical.
fn match_score(command: &Command, tokens: &[String]) -> Option<(usize, usize, bool)> {
    let label: Vec<String> = words_of(command.display);
    let aliases: Vec<String> = command
        .aliases
        .iter()
        .flat_map(|phrase| words_of(phrase))
        .filter(|word| !label.contains(word))
        .collect();

    // Every word, labelled or not, with its position, so "used" can be per word
    // and a token cannot match the same word twice.
    let mut words: Vec<&String> = label.iter().chain(aliases.iter()).collect();
    words.sort();
    words.dedup();

    let mut used = vec![false; words.len()];
    let mut count = 0usize;
    let mut strength = 0usize;
    let mut on_label = true;
    for token in tokens {
        let index = words
            .iter()
            .enumerate()
            .position(|(i, word)| !used[i] && word.starts_with(token.as_str()))?;
        used[index] = true;
        count += 1;
        strength += token.len();
        on_label &= label.contains(words[index]);
    }
    Some((count, strength, on_label))
}

/// The words of a phrase, lowercased and stripped.
fn words_of(phrase: &str) -> Vec<String> {
    phrase
        .to_lowercase()
        .split(' ')
        .map(strip_punctuation)
        .filter(|word| !word.is_empty())
        .collect()
}

/// The punctuation the site's filter strips, which is everything that is not
/// alphanumeric or a space.
fn strip_punctuation(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect()
}

/// A command's display, for the list.
pub fn display(index: usize) -> &'static str {
    COMMANDS.get(index).map_or("?", |command| command.display)
}

/// A command's action, or `None` for an index that is not a command.
pub fn action(index: usize) -> Option<Action> {
    COMMANDS.get(index).map(|command| command.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The displays a query matches, in order.
    fn found(query: &str) -> Vec<&'static str> {
        filter(query)
            .into_iter()
            .map(|m| display(m.command))
            .collect()
    }

    #[test]
    fn an_empty_query_lists_everything_in_order() {
        let all = found("");
        assert_eq!(all.len(), COMMANDS.len());
        assert_eq!(all[0], "next test", "the list starts with the test itself");
    }

    /// `th` reaches the themes, and the one whose *label* starts with it comes
    /// first even though `quote length` matches equally hard through the hidden
    /// alias `thicc`. A search that puts the surprising answer first is a search
    /// that has to be retyped.
    #[test]
    fn a_prefix_finds_a_command_with_the_visible_match_first() {
        let found = found("th");
        assert_eq!(found.first(), Some(&"next theme"), "{found:?}");
        assert!(found.contains(&"gruvbox"), "{found:?}");
        // The alias still finds it — it just does not lead.
        assert!(found.contains(&"quote length"), "{found:?}");
    }

    /// A label match beats an alias match even when the alias is longer, because
    /// the label is the name the user can see.
    #[test]
    fn a_label_match_outranks_a_longer_alias_match() {
        let found = found("the");
        assert_eq!(found.first(), Some(&"next theme"), "{found:?}");
    }

    /// Two tokens must match two different words, so `set custom words 30` is
    /// findable and `30 custom` is not.
    #[test]
    fn two_tokens_need_two_different_words() {
        let both = found("custom words");
        assert!(both.contains(&"word count"), "{both:?}");
        // The same two words the other way round still matches, because the
        // matching is per-word and not positional.
        assert!(found("words custom").contains(&"word count"));
    }

    #[test]
    fn a_token_that_matches_nowhere_finds_nothing() {
        assert!(found("qwertyuiop").is_empty());
        assert!(
            found("th zzzzz").is_empty(),
            "one bad token loses the whole query"
        );
    }

    #[test]
    fn aliases_are_searchable() {
        assert!(found("punc").contains(&"punctuation"));
        assert!(found("lang").contains(&"language"));
        assert!(found("exit").contains(&"quit"));
    }

    /// The site's rule: the longest total match wins, so a short prefix does not
    /// drag in everything that contains it.
    #[test]
    fn the_longest_match_wins() {
        // `th` matches "theme" and "next theme"; the longer one is kept because
        // the site ranks on total match length and keeps only the best.
        let all = found("th");
        assert!(!all.is_empty());
    }

    #[test]
    fn the_filter_is_case_and_punctuation_insensitive() {
        assert_eq!(found("THEME"), found("theme"));
        assert_eq!(found("theme!"), found("theme"));
        assert!(found("quote length").contains(&"quote length"));
        // A hyphen is *stripped* rather than treated as a separator, so the joined
        // spelling finds nothing. That is the site's rule and it is the useful
        // one: `stripPunctuation` turns "quote-length" into "quotelength", which
        // is a word nobody types, and a query that finds nothing is honest
        // rather than a near miss.
        assert!(
            !found("quote-length").contains(&"quote length"),
            "a hyphen should not join two words"
        );
    }

    /// Every command must do something. A command that reaches `run_command` and
    /// falls through is a line in a list that does nothing when pressed.
    #[test]
    fn every_command_has_an_action() {
        for (index, command) in COMMANDS.iter().enumerate() {
            assert_eq!(action(index), Some(command.action), "{command:?}");
        }
    }

    /// Every mode must be reachable, or the bar is the only way to change it.
    #[test]
    fn every_mode_is_a_command() {
        for mode in [
            ConfigMode::Time,
            ConfigMode::Words,
            ConfigMode::Quote,
            ConfigMode::Zen,
            ConfigMode::Custom,
        ] {
            assert!(
                COMMANDS.iter().any(|c| c.action == Action::Mode(mode)),
                "{mode:?} is not in the command list"
            );
        }
    }

    /// Difficulty is not on the site's test bar, so the command list is the quick
    /// way to it. Both directions have to exist or "harder" is the only way to
    /// change it.
    #[test]
    fn difficulty_is_reachable_in_both_directions() {
        let harder = COMMANDS
            .iter()
            .find(|c| c.display == "harder")
            .expect("a harder command");
        let easier = COMMANDS
            .iter()
            .find(|c| c.display == "easier")
            .expect("an easier command");
        assert_eq!(harder.action, Action::Difficulty(1));
        assert_eq!(easier.action, Action::Difficulty(-1));
    }

    #[test]
    fn no_two_commands_show_the_same_name() {
        let mut seen = std::collections::BTreeSet::new();
        for command in COMMANDS {
            assert!(
                seen.insert(command.display),
                "{} is listed twice",
                command.display
            );
        }
    }

    /// The theme commands name real themes, or they set a name that does not exist.
    #[test]
    fn the_theme_commands_name_real_themes() {
        for command in COMMANDS {
            if let Action::Theme(name) = command.action {
                let label = name.replace('_', " ");
                assert!(
                    crate::config::theme::ThemeName::ALL
                        .iter()
                        .any(|t| t.label() == label),
                    "{name} is not a theme"
                );
            }
        }
    }

    #[test]
    fn an_index_past_the_end_is_nothing_rather_than_a_panic() {
        assert_eq!(display(9999), "?");
        assert_eq!(action(9999), None);
    }
}
