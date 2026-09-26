//! Keybind table.

use serde::{Deserialize, Serialize};

use crate::action::Action;

/// Logical action names used in `config.toml`.
///
/// The defaults are deliberately *not* bare letters. A single-letter binding
/// makes that letter untypeable, and on the typing screen every letter is text:
/// `t` starts "the", `,` is a punctuation test, `q` is a word. Commands that
/// must work mid-test therefore carry a modifier or a function key, which is
/// also how the website avoids the same problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Binding {
    Quit,
    Up,
    Down,
    Left,
    Right,
    Select,
    Back,
    Settings,
    StartTest,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keybinds {
    pub quit: Vec<String>,
    pub up: Vec<String>,
    pub down: Vec<String>,
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub select: Vec<String>,
    pub back: Vec<String>,
    pub settings: Vec<String>,
    pub start_test: Vec<String>,
    pub restart: Vec<String>,
}

impl Default for Keybinds {
    fn default() -> Self {
        Self {
            // `ctrl+c` is the only quit binding, because it can never be text.
            quit: vec!["ctrl+c".into()],
            up: vec!["up".into(), "k".into()],
            down: vec!["down".into(), "j".into()],
            left: vec!["left".into(), "h".into()],
            right: vec!["right".into(), "l".into()],
            select: vec!["enter".into()],
            back: vec!["esc".into()],
            settings: vec!["f2".into()],
            start_test: vec!["ctrl+t".into()],
            restart: vec!["ctrl+r".into()],
        }
    }
}

/// The bindings shipped before the "a bare letter is untypeable" rule existed.
///
/// A config file written by an older build carries these verbatim, which means
/// it silently shadows every later change to the defaults. [`Keybinds::migrate`]
/// looks for exactly this table.
pub fn legacy_v0() -> Keybinds {
    Keybinds {
        quit: vec!["q".into(), "ctrl+c".into()],
        up: vec!["up".into(), "k".into()],
        down: vec!["down".into(), "j".into()],
        left: vec!["left".into(), "h".into()],
        right: vec!["right".into(), "l".into()],
        select: vec!["space".into()],
        back: vec!["esc".into(), "enter".into()],
        settings: vec![",".into()],
        start_test: vec!["t".into()],
        restart: vec!["r".into()],
    }
}

impl Keybinds {
    /// Upgrades the pre-1.0 defaults to the current ones.
    ///
    /// Only a group that still matches the old default *exactly* is replaced, so
    /// an edit the user made survives. The unavoidable cost is a config that was
    /// never touched but happens to say `restart = ["r"]`: that is
    /// indistinguishable from having asked for it, and is treated as asked for.
    pub fn migrate(&mut self) {
        let legacy = legacy_v0();
        let current = Keybinds::default();
        if self.quit == legacy.quit {
            self.quit = current.quit;
        }
        if self.up == legacy.up {
            self.up = current.up;
        }
        if self.down == legacy.down {
            self.down = current.down;
        }
        if self.left == legacy.left {
            self.left = current.left;
        }
        if self.right == legacy.right {
            self.right = current.right;
        }
        if self.select == legacy.select {
            self.select = current.select;
        }
        if self.back == legacy.back {
            self.back = current.back;
        }
        if self.settings == legacy.settings {
            self.settings = current.settings;
        }
        if self.start_test == legacy.start_test {
            self.start_test = current.start_test;
        }
        if self.restart == legacy.restart {
            self.restart = current.restart;
        }
    }
}

impl Keybinds {
    /// All bindings, paired with the action they trigger.
    pub fn all(&self) -> Vec<(Action, &Vec<String>)> {
        vec![
            (Action::Quit, &self.quit),
            (Action::Up, &self.up),
            (Action::Down, &self.down),
            (Action::Left, &self.left),
            (Action::Right, &self.right),
            (Action::Select, &self.select),
            (Action::Back, &self.back),
            (Action::Settings, &self.settings),
            (Action::StartTest, &self.start_test),
            (Action::Restart, &self.restart),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_binds_quit_to_ctrl_c() {
        let keybinds = Keybinds::default();
        assert!(keybinds.quit.contains(&"ctrl+c".to_string()));
    }

    #[test]
    fn no_mid_test_binding_is_a_bare_letter() {
        // A bare letter binding makes that letter untypeable, and every letter
        // has to be typeable on the typing screen. Only the bindings that have to
        // work *during* a test are checked: the menu navigation keys are never
        // read there, and `hjkl` is worth having on the settings screen.
        let keybinds = Keybinds::default();
        for keys in [
            &keybinds.quit,
            &keybinds.settings,
            &keybinds.start_test,
            &keybinds.restart,
        ] {
            for key in keys {
                assert!(
                    !key.chars().all(|c| !c.is_ascii_alphanumeric()) || key.chars().count() > 1,
                    "`{key}` is a bare key and would be untypeable"
                );
            }
        }
    }

    #[test]
    fn menu_navigation_keeps_its_vim_keys() {
        let keybinds = Keybinds::default();
        assert!(keybinds.left.contains(&"h".to_string()));
        assert!(keybinds.right.contains(&"l".to_string()));
        assert!(keybinds.up.contains(&"k".to_string()));
        assert!(keybinds.down.contains(&"j".to_string()));
    }

    #[test]
    fn every_action_has_a_binding() {
        let keybinds = Keybinds::default();
        let actions: Vec<Action> = keybinds.all().into_iter().map(|(a, _)| a).collect();
        for action in [
            Action::Quit,
            Action::Up,
            Action::Down,
            Action::Left,
            Action::Right,
            Action::Select,
            Action::Back,
            Action::Settings,
            Action::StartTest,
            Action::Restart,
        ] {
            assert!(actions.contains(&action), "{action:?} is unreachable");
        }
    }

    #[test]
    fn migration_replaces_the_untypeable_legacy_defaults() {
        let mut keybinds = legacy_v0();
        keybinds.migrate();
        assert_eq!(keybinds, Keybinds::default());
    }

    #[test]
    fn migration_keeps_a_binding_the_user_actually_chose() {
        let mut keybinds = legacy_v0();
        keybinds.restart = vec!["ctrl+y".into()];
        keybinds.migrate();
        assert_eq!(keybinds.restart, vec!["ctrl+y".to_string()]);
        // the untouched groups still move
        assert_eq!(keybinds.settings, vec!["f2".to_string()]);
    }

    #[test]
    fn migrating_twice_changes_nothing() {
        let mut keybinds = legacy_v0();
        keybinds.migrate();
        let once = keybinds.clone();
        keybinds.migrate();
        assert_eq!(keybinds, once);
    }
}
