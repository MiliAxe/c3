use crossterm::event::{KeyCode, KeyEvent};
use std::collections::hash_map::Iter;
use std::collections::HashMap;
use std::convert::From;
use std::sync::Arc;

pub struct Keymap<'a> {
    pub description: String,
    pub action: Arc<dyn Fn(&mut super::TuiApp<'a>) -> super::HandlerOperation + 'a>,
}

pub struct KeymapManager<'a> {
    pub keymaps: HashMap<KeyEvent, Keymap<'a>>,
}

impl<'a> KeymapManager<'a> {
    pub fn new() -> Self {
        KeymapManager {
            keymaps: HashMap::new(),
        }
    }

    pub fn add(&mut self, key: KeyEvent, keymap: Keymap<'a>) -> () {
        self.keymaps.insert(key, keymap);
    }

    pub fn get_all(&self) -> Iter<'_, KeyEvent, Keymap<'a>> {
        self.keymaps.iter()
    }

    pub fn do_action(&self, key: KeyEvent, tui_app: &mut super::TuiApp<'a>) -> () {
        let keymap = self.keymaps.get(&key);
        if let Some(x) = keymap {
            (x.action)(tui_app);
        }
    }

    pub fn get_action(
        &self,
        key: KeyEvent,
    ) -> Option<Arc<dyn Fn(&mut super::TuiApp<'a>) -> super::HandlerOperation + 'a>> {
        self.keymaps.get(&key).map(|km| km.action.clone())
    }
}

impl<'a> From<Vec<(KeyEvent, Keymap<'a>)>> for KeymapManager<'a> {
    fn from(value: Vec<(KeyEvent, Keymap<'a>)>) -> Self {
        let keymaps: HashMap<KeyEvent, Keymap<'a>> = value.into_iter().collect();
        KeymapManager { keymaps }
    }
}

#[macro_export]
macro_rules! keymap_entry {
    ($key:expr, $desc:expr, $action:expr) => {
        (
            $key,
            Keymap {
                description: String::from($desc),
                action: Arc::new($action),
            },
        )
    };
}

pub fn key_event_to_string(key_event: &KeyEvent) -> String {
    let modifier_str = match key_event.modifiers {
        crossterm::event::KeyModifiers::CONTROL => "Ctrl+",
        crossterm::event::KeyModifiers::ALT => "Alt+",
        crossterm::event::KeyModifiers::SHIFT => "Shift+",
        crossterm::event::KeyModifiers::NONE => "",
        _ => "",
    };
    let keycode_str = match key_event.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Esc => "Escape".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        other => format!("{:?}", other),
    };

    format!("{}{}", modifier_str, keycode_str)
}
