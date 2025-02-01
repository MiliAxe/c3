use crossterm::event::{KeyEvent, KeyModifiers};
use std::collections::hash_map::Iter;
use std::collections::HashMap;
use std::convert::From;

pub struct Keymap {
    key: KeyEvent,
    modifier: KeyModifiers,
    description: String,
    action: Box<dyn Fn()>,
}

pub struct KeymapManager {
    keymaps: HashMap<KeyEvent, Keymap>,
}

impl KeymapManager {
    pub fn new() -> Self {
        KeymapManager {
            keymaps: HashMap::new(),
        }
    }

    pub fn add(&mut self, keymap: Keymap) -> () {
        self.keymaps.insert(keymap.key, keymap);
    }

    pub fn get_all(&self) -> Iter<'_, KeyEvent, Keymap> {
        self.keymaps.iter()
    }
    
    pub fn do_action(&self, key: KeyEvent) -> () {
        let keymap = self.keymaps.get(&key);
        if let Some(x) = keymap { (x.action)() }
    }
}

impl From<Vec<(KeyEvent, Keymap)>> for KeymapManager {
    fn from(value: Vec<(KeyEvent, Keymap)>) -> Self {
        let keymaps: HashMap<KeyEvent, Keymap> = value.into_iter().collect();
        KeymapManager { keymaps }
    }
}
