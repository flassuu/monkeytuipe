//! Keybind table.

use serde::{Deserialize, Serialize};

use crate::action::Action;

/// Logical action names used in `config.toml`.
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
    fn default_binds_quit_to_q_and_ctrl_c() {
        let keybinds = Keybinds::default();
        assert!(keybinds.quit.contains(&"q".to_string()));
        assert!(keybinds.quit.contains(&"ctrl+c".to_string()));
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
