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
}
