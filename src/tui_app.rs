// vim:fileencoding=utf-8:foldmethod=marker
// imports {{{
use clap::Parser;
use crossterm::{
    event::{
        self,
        Event::Key,
        KeyCode::{self, Char},
        KeyEvent, KeyModifiers,
    },
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::getpid;
use ratatui::{prelude::*, widgets::*};
use std::io::Write;
use std::ops::Not;
use std::{
    io::{self, BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    rc::Rc,
    str,
};
use tui_textarea::{CursorMove, Input, TextArea};
mod help;
mod keymap;
mod potato;
mod todo_buffer;
use todo_buffer::TodoBuffer;
mod tree_search;
use c3::{
    date,
    todo_app::{fzf_search::fzf_search, App, Restriction, Schedule, Todo},
};
pub use tree_search::TreeSearch;

use help::HelpPage;
use keymap::{key_event_to_string, Keymap, KeymapManager};
use potato::Potato;
use std::sync::Arc;

use crate::keymap_entry;
// }}}

#[derive(Debug)]
pub enum HandlerOperation {
    Nothing,
    Restart,
}

#[derive(Debug, PartialEq)]
enum EditorOperation {
    Cancel,
    Submit,
    Input,
    Delete(String),
}

#[derive(Default, PartialEq)]
enum Mode {
    #[default]
    Normal,
    Editing,
}

pub struct TuiApp<'a> {
    tree_search: TreeSearch,
    todo_buffer: TodoBuffer,
    last_restriction: Option<Restriction>,
    show_right: bool,
    show_help: bool,
    help_page: HelpPage,
    normal_keymaps: KeymapManager<'a>,
    mode: Mode,
    on_submit: Option<fn(&mut Self, String) -> ()>,
    on_delete: Option<fn(&mut Self, String, String) -> ()>,
    on_input: Option<fn(&mut Self, String) -> ()>,
    args: TuiArgs,
    potato_module: Potato,
    textarea: TextArea<'a>,
    todo_app: &'a mut App,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct TuiArgs {
    /// Alternative way of rendering, render minimum amount of todos
    #[arg(long)]
    minimal_render: bool,

    /// String behind highlighted todo in TUI mode
    #[arg(short='H', long, default_value_t=String::from(">>"))]
    highlight_string: String,

    /// Enable TUI module at startup
    #[arg(short = 'm', long)]
    enable_module: bool,

    /// Don't use glow for notes
    #[arg(short = 'G', long)]
    no_glow: bool,
}

impl<'a> TuiApp<'a> {
    #[inline]
    pub fn new(app: &'a mut App, args: TuiArgs) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        let normal_keymaps = Self::create_normal_keymaps();
        let app_help_page = TuiApp::get_default_help_page(&normal_keymaps);

        TuiApp {
            tree_search: Default::default(),
            todo_buffer: Default::default(),
            todo_app: app,
            args,
            textarea,
            potato_module: Default::default(),
            on_submit: None,
            on_input: None,
            on_delete: None,
            show_right: true,
            help_page: app_help_page,
            normal_keymaps,
            show_help: false,
            mode: Default::default(),
            last_restriction: None,
        }
    }

    fn create_normal_keymaps() -> KeymapManager<'a> {
        let keymaps_vec = vec![
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
                "Toggle help window",
                |app: &mut TuiApp| {
                    app.show_help = !app.show_help;
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
                "Prepend todo",
                |app: &mut TuiApp| {
                    app.prepend_prompt();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
                "Suspend application (Unix)",
                |_app: &mut TuiApp| {
                    let _ = shutdown();
                    #[cfg(unix)]
                    {
                        let _ = kill(getpid(), Signal::SIGTSTP);
                        HandlerOperation::Restart
                    }
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
                "Open file browser",
                |app: &mut TuiApp| {
                    app.nnn_open();
                    HandlerOperation::Restart
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                "Remove todo and yank",
                |app: &mut TuiApp| {
                    app.todo_app.remove_todo();
                    if let Some(todo) = app.todo_app.removed_todos.pop() {
                        app.todo_buffer.yank(todo);
                    }
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                "Toggle daily",
                |app: &mut TuiApp| {
                    app.todo_app.toggle_current_daily();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE),
                "Toggle weekly",
                |app: &mut TuiApp| {
                    app.todo_app.toggle_current_weekly();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE),
                "Schedule prompt",
                |app: &mut TuiApp| {
                    app.schedule_prompt();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
                "Reminder prompt",
                |app: &mut TuiApp| {
                    app.reminder_prompt();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('M'), KeyModifiers::NONE),
                "Toggle schedule",
                |app: &mut TuiApp| {
                    if let Some(todo) = app.todo_app.todo_mut() {
                        todo.toggle_schedule();
                    }
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE),
                "Toggle show done",
                |app: &mut TuiApp| {
                    app.todo_app.toggle_show_done();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE),
                "Priority prompt",
                |app: &mut TuiApp| {
                    app.priority_prompt();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('%'), KeyModifiers::NONE),
                "Schedule restriction prompt",
                |app: &mut TuiApp| {
                    app.schedule_restriction_prompt();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                "Yank todo",
                |app: &mut TuiApp| {
                    let todo = app.todo_app.todo().cloned();
                    app.todo_buffer.yank(todo);
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
                "Paste todo",
                |app: &mut TuiApp| {
                    if let Some(todo) = app.todo_buffer.get() {
                        let list = app.todo_app.current_list_mut();
                        list.push(todo);
                        app.todo_app.index = list.reorder_last();
                    }
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
                "Increase day by 1",
                |app: &mut TuiApp| {
                    app.todo_app.increase_day_by(1);
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE),
                "Increase day by -1",
                |app: &mut TuiApp| {
                    app.todo_app.increase_day_by(-1);
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
                "Append todo",
                |app: &mut TuiApp| {
                    app.nnn_append_todo();
                    HandlerOperation::Restart
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('O'), KeyModifiers::NONE),
                "Output todo",
                |app: &mut TuiApp| {
                    app.nnn_output_todo();
                    HandlerOperation::Restart
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                "Move down",
                |app: &mut TuiApp| {
                    app.todo_app.go_down();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                "Move down",
                |app: &mut TuiApp| {
                    app.todo_app.go_down();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
                "Move up",
                |app: &mut TuiApp| {
                    app.todo_app.go_up();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
                "Move up",
                |app: &mut TuiApp| {
                    app.todo_app.go_up();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
                "Move right",
                |app: &mut TuiApp| {
                    app.todo_app.add_dependency_traverse_down();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
                "Move right",
                |app: &mut TuiApp| {
                    app.todo_app.add_dependency_traverse_down();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                "Traverse down",
                |app: &mut TuiApp| {
                    app.todo_app.traverse_down();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
                "Move left",
                |app: &mut TuiApp| {
                    app.todo_app.traverse_up();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                "Move left",
                |app: &mut TuiApp| {
                    app.todo_app.traverse_up();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
                "Go to top",
                |app: &mut TuiApp| {
                    app.todo_app.index = 0;
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
                "Go to top",
                |app: &mut TuiApp| {
                    app.todo_app.index = 0;
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
                "Go to bottom",
                |app: &mut TuiApp| {
                    app.todo_app.index = app.todo_app.bottom();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE),
                "Go to bottom",
                |app: &mut TuiApp| {
                    app.todo_app.index = app.todo_app.bottom();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
                "Write",
                |app: &mut TuiApp| {
                    let _ = app.write();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE),
                "Move current down",
                |app: &mut TuiApp| {
                    app.todo_app.move_current_down();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE),
                "Move current up",
                |app: &mut TuiApp| {
                    app.todo_app.move_current_up();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
                "Toggle right panel",
                |app: &mut TuiApp| {
                    app.show_right = !app.show_right;
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE),
                "Toggle module",
                |app: &mut TuiApp| {
                    app.args.enable_module = !app.args.enable_module;
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('>'), KeyModifiers::NONE),
                "Edit or add note",
                |app: &mut TuiApp| {
                    app.todo_app.edit_or_add_note();
                    HandlerOperation::Restart
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
                "Add dependency",
                |app: &mut TuiApp| {
                    app.todo_app.add_dependency();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('D'), KeyModifiers::NONE),
                "Remove todo",
                |app: &mut TuiApp| {
                    app.todo_app.remove_todo();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
                "Read",
                |app: &mut TuiApp| {
                    app.todo_app.read();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('T'), KeyModifiers::NONE),
                "Remove current dependent",
                |app: &mut TuiApp| {
                    app.todo_app.remove_current_dependent();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                "Toggle current done",
                |app: &mut TuiApp| {
                    app.todo_app.toggle_current_done();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
                "Next search result",
                |app: &mut TuiApp| {
                    app.tree_search.next();
                    app.tree_search.set_to_app(app.todo_app);
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
                "Search prompt",
                |app: &mut TuiApp| {
                    app.search_prompt();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('\''), KeyModifiers::NONE),
                "Tree search prompt",
                |app: &mut TuiApp| {
                    app.tree_search_prompt();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE),
                "Append todo at first",
                |app: &mut TuiApp| {
                    app.append_prompt();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
                "Edit todo",
                |app: &mut TuiApp| {
                    app.edit_prompt(false);
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE),
                "Edit todo (start)",
                |app: &mut TuiApp| {
                    app.edit_prompt(true);
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                "Batch edit messages",
                |app: &mut TuiApp| {
                    app.todo_app.batch_editor_messages();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('~'), KeyModifiers::NONE),
                "Go to root",
                |app: &mut TuiApp| {
                    app.todo_app.go_root();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
                "Quit and save prompt",
                |app: &mut TuiApp| {
                    app.quit_save_prompt();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
                "Batch edit messages",
                |app: &mut TuiApp| {
                    app.todo_app.batch_editor_messages();
                    HandlerOperation::Restart
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
                "Skip potato module",
                |app: &mut TuiApp| {
                    app.potato_module.skip();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('H'), KeyModifiers::NONE),
                "Increase potato timer",
                |app: &mut TuiApp| {
                    app.potato_module.increase_timer();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
                "Toggle potato pause",
                |app: &mut TuiApp| {
                    app.potato_module.toggle_pause();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE),
                "Quit potato module",
                |app: &mut TuiApp| {
                    app.potato_module.quit();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE),
                "Decrease potato timer",
                |app: &mut TuiApp| {
                    app.potato_module.decrease_timer();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
                "Restart potato module",
                |app: &mut TuiApp| {
                    app.potato_module.restart();
                    HandlerOperation::Nothing
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('F'), KeyModifiers::NONE),
                "FZF search",
                |app: &mut TuiApp| {
                    fzf_search(app.todo_app);
                    HandlerOperation::Restart
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
                "Increase pomodoro",
                |app: &mut TuiApp| {
                    app.potato_module.increase_pomodoro();
                    HandlerOperation::Restart
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE),
                "Decrease pomodoro",
                |app: &mut TuiApp| {
                    app.potato_module.decrease_pomodoro();
                    HandlerOperation::Restart
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE),
                "Next potato module",
                |app: &mut TuiApp| {
                    app.potato_module.next();
                    HandlerOperation::Restart
                }
            ),
            keymap_entry!(
                KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE),
                "Previous potato module",
                |app: &mut TuiApp| {
                    app.potato_module.prev();
                    HandlerOperation::Restart
                }
            ),
        ];
        KeymapManager::from(keymaps_vec)
    }

    fn get_default_help_page(keymap: &KeymapManager) -> HelpPage {
        let mut help_page = HelpPage::default();

        for (key, keymap) in keymap.get_all() {
            help_page.add_entry(
                key_event_to_string(key).as_str(),
                keymap.description.clone().as_str(),
            );
        }

        help_page
    }

    #[inline]
    pub fn title(&mut self) -> String {
        let changed_str = if self.todo_app.current_list().changed {
            "*"
        } else {
            ""
        };
        let size = self
            .todo_app
            .current_list()
            .len(self.todo_app.get_restriction());
        let todo_string = format!("Todos ({size}){changed_str}");

        if let Some(parent) = self.todo_app.parent() {
            format!("{todo_string} {}", parent.message)
        } else {
            todo_string
        }
    }

    #[inline]
    pub fn quit(&self) -> io::Result<()> {
        shutdown()?;
        std::process::exit(0);
    }

    #[inline]
    pub fn set_text_mode(
        &mut self,
        on_submit: fn(&mut Self, String) -> (),
        title: &'a str,
        placeholder: &str,
    ) {
        self.on_input = None;
        self.on_delete = None;
        self.on_submit = Some(on_submit);
        self.turn_on_text_mode(title, placeholder);
    }

    #[inline]
    pub fn set_responsive_text_mode(
        &mut self,
        on_input: fn(&mut Self, String) -> (),
        title: &'a str,
        placeholder: &str,
    ) {
        self.on_input = Some(on_input);
        self.turn_on_text_mode(title, placeholder);
    }

    #[inline(always)]
    fn turn_on_text_mode(&mut self, title: &'a str, placeholder: &str) {
        self.textarea.set_placeholder_text(placeholder);
        self.textarea.set_block(default_block(title));
        self.mode = Mode::Editing;
    }

    #[inline(always)]
    fn turn_off_text_mode(&mut self) {
        self.textarea.delete_line_by_head();
        self.textarea.delete_line_by_end();
        self.mode = Mode::Normal;
    }

    #[inline]
    pub fn search_prompt(&mut self) {
        const TITLE: &str = "Search todo";
        const PLACEHOLDER: &str = "Enter search query";
        self.last_restriction = Some(Rc::clone(self.todo_app.get_restriction()));
        self.on_submit = None;
        self.set_responsive_text_mode(Self::on_search, TITLE, PLACEHOLDER);
        self.on_delete = Some(Self::on_search_delete);
    }

    #[inline]
    pub fn tree_search_prompt(&mut self) {
        self.set_text_mode(
            Self::on_tree_search,
            "Search the whole tree for todo",
            "Enter search query",
        )
    }

    #[inline]
    fn on_search(&mut self, query: String) {
        self.todo_app.set_restriction_with_last(
            Rc::new(move |todo| todo.matches(query.as_str())),
            self.last_restriction.clone(),
        )
    }

    #[inline]
    fn on_priority_delete(&mut self, new: String, old: String) {
        if new.is_empty() {
            if let Some(restriction) = self.last_restriction.clone() {
                self.todo_app.set_restriction(restriction)
            }
        }
        if old.is_empty() {
            self.todo_app.update_show_done_restriction()
        }
    }

    #[inline]
    fn on_search_delete(&mut self, str: String, old: String) {
        if old.is_empty() {
            self.todo_app.update_show_done_restriction()
        } else {
            self.on_search(str)
        }
    }

    #[inline]
    fn on_tree_search(&mut self, query: String) {
        let current_not_matches = self
            .todo_app
            .todo()
            .map_or(true, |todo| !todo.matches(&query));

        self.tree_search.search(
            query,
            self.todo_app.current_list(),
            Rc::clone(self.todo_app.get_restriction()),
        );
        if current_not_matches {
            self.tree_search.next();
            self.tree_search.set_to_app(self.todo_app);
        }
    }

    #[inline]
    pub fn schedule_prompt(&mut self) {
        self.set_text_mode(Self::on_schedule, "Change schedule day", "");
    }

    #[inline]
    fn on_schedule(&mut self, str: String) {
        let day = str.parse::<u64>().ok();
        if day.is_none() {
            return;
        }
        if let Some(todo) = self.todo_app.todo_mut() {
            todo.enable_day(day.unwrap() as i64);
        }
    }

    #[inline]
    pub fn reminder_prompt(&mut self) {
        self.set_text_mode(Self::on_reminder, "Date reminder", "");
    }

    fn nnn_paths() -> Option<impl Iterator<Item = PathBuf>> {
        let mut output = Command::new("nnn")
            .args(["-p", "-"])
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("Failed to start nnn.");

        let exit_status = output.wait().expect("Failed to wait on nnn.");
        if exit_status.success() {
            let reader = BufReader::new(output.stdout.unwrap());
            return Some(reader.lines().map(|x| PathBuf::from(x.unwrap_or_default())));
        }
        None
    }

    #[inline]
    pub fn nnn_append_todo(&mut self) {
        if let Some(paths) = Self::nnn_paths() {
            for path in paths {
                self.todo_app.append_list_from_path(&path);
            }
        }
    }

    pub fn nnn_open(&mut self) {
        if let Some(paths) = Self::nnn_paths() {
            for path in paths {
                self.todo_app.open_path(path);
            }
        }
    }

    #[inline]
    pub fn nnn_output_todo(&mut self) {
        if let Some(paths) = Self::nnn_paths() {
            for path in paths {
                let _ = self.todo_app.output_list_to_path(&path);
            }
        }
    }

    #[inline]
    fn on_reminder(&mut self, str: String) {
        if let Ok(date) = date::parse_user_input(&str) {
            if let Some(todo) = self.todo_app.todo_mut() {
                todo.schedule = Some(Schedule::new_reminder(date));
                self.todo_app.reorder_current();
            }
        }
    }

    #[inline]
    pub fn edit_prompt(&mut self, start: bool) {
        if let Some(message) = &self.todo_app.todo().map(|todo| todo.message.clone()) {
            self.set_text_mode(Self::on_edit_todo, "Edit todo", message);
            self.textarea.insert_str(message);
            if start {
                self.textarea.move_cursor(CursorMove::Head);
            }
        }
    }

    #[inline]
    pub fn prepend_prompt(&mut self) {
        self.set_text_mode(Self::on_append_todo, "Add todo", "Enter the todo message");
    }

    #[inline]
    pub fn priority_prompt(&mut self) {
        const TITLE: &str = "Limit priority";
        const PLACEHOLDER: &str = "Enter priority to show";
        self.last_restriction = Some(self.todo_app.get_restriction().clone());
        self.set_text_mode(Self::on_priority_prompt, TITLE, PLACEHOLDER);
        self.set_responsive_text_mode(Self::on_priority_prompt, TITLE, PLACEHOLDER);
        self.on_delete = Some(Self::on_priority_delete);
    }

    #[inline]
    pub fn schedule_restriction_prompt(&mut self) {
        const TITLE: &str = "Limit schedule";
        const PLACEHOLDER: &str = "Enter schedule to show";
        self.last_restriction = Some(self.todo_app.get_restriction().clone());
        self.set_text_mode(Self::on_schedule_prompt, TITLE, PLACEHOLDER);
        self.set_responsive_text_mode(Self::on_schedule_prompt, TITLE, PLACEHOLDER);
        self.on_delete = Some(Self::on_priority_delete);
    }

    #[inline]
    pub fn append_prompt(&mut self) {
        self.set_text_mode(
            Self::on_prepend_todo,
            "Add todo at first",
            "Enter the todo message",
        );
    }

    #[inline]
    pub fn quit_save_prompt(&mut self) {
        if self.todo_app.current_list().changed || self.todo_app.is_changed() {
            self.set_text_mode(
                Self::on_save_prompt,
                "You have done changes. You wanna save? [n: no, y: yes, c: cancel] (default: n)",
                "N/y/c",
            );
        } else {
            let _ = self.quit();
        }
    }

    #[inline]
    fn on_priority_prompt(&mut self, str: String) {
        if str.is_empty() {
            return self.todo_app.update_show_done_restriction();
        }
        let priority = str.parse();
        if let Ok(priority) = priority {
            self.todo_app.set_restriction_with_last(
                Rc::new(move |todo| todo.priority() == priority),
                self.last_restriction.clone(),
            )
        }
    }

    #[inline]
    fn on_schedule_prompt(&mut self, str: String) {
        if str.is_empty() {
            return self.todo_app.update_show_done_restriction();
        }
        let schedule_day = str.parse();
        if let Ok(schedule_day) = schedule_day {
            self.todo_app.set_restriction_with_last(
                Rc::new(move |todo| {
                    todo.schedule
                        .as_ref()
                        .map_or(0, |sch| if sch.is_reminder() { 0 } else { sch.days() })
                        == schedule_day
                }),
                self.last_restriction.clone(),
            )
        }
    }

    #[inline]
    fn on_save_prompt(&mut self, str: String) {
        let lower = str.to_lowercase();
        if lower.starts_with('y') {
            let _ = self.todo_app.write();
        } else if lower.starts_with('c') {
            return;
        }
        let _ = self.quit();
    }

    #[inline]
    fn on_append_todo(&mut self, str: String) {
        self.todo_app.append(str);
    }

    #[inline]
    fn on_prepend_todo(&mut self, str: String) {
        self.todo_app.prepend(str);
    }

    #[inline]
    fn on_edit_todo(&mut self, str: String) {
        if !str.is_empty() {
            if let Some(todo) = self.todo_app.todo_mut() {
                todo.message = str;
            }
        }
    }

    #[inline(always)]
    fn current_textarea_message(&self) -> String {
        self.textarea.lines()[0].clone()
    }

    #[inline]
    fn handle_text_input(&mut self) -> io::Result<HandlerOperation> {
        let operation = self.editor()?;
        match operation {
            EditorOperation::Input => {
                if let Some(on_input) = self.on_input {
                    let message = self.current_textarea_message();
                    on_input(self, message);
                }
            }
            EditorOperation::Submit => {
                if let Some(on_submit) = self.on_submit {
                    let message = self.current_textarea_message();
                    on_submit(self, message);
                }
                self.turn_off_text_mode();
            }
            EditorOperation::Cancel => {
                self.turn_off_text_mode();
            }
            EditorOperation::Delete(before_delete) => {
                if let Some(on_delete) = self.on_delete {
                    let message = self.current_textarea_message();
                    on_delete(self, message, before_delete);
                }
            }
        }
        Ok(HandlerOperation::Nothing)
    }

    #[inline]
    fn editor(&mut self) -> io::Result<EditorOperation> {
        let event = event::read()?;
        if let Key(key) = event {
            match key.code {
                KeyCode::Esc => return Ok(EditorOperation::Cancel),
                KeyCode::Enter => return Ok(EditorOperation::Submit),
                Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                    let before_delete = self.current_textarea_message();
                    self.textarea.delete_line_by_head();
                    return Ok(EditorOperation::Delete(before_delete));
                }
                KeyCode::Backspace => {
                    let before_delete = self.current_textarea_message();
                    self.textarea.delete_char();
                    return Ok(EditorOperation::Delete(before_delete));
                }
                _ => {}
            }
        }
        let input: Input = event.into();
        self.textarea.input(input);
        Ok(EditorOperation::Input)
    }

    #[inline]
    pub fn handle_key_and_return_operation(&mut self) -> io::Result<HandlerOperation> {
        let input_handler = match self.mode {
            Mode::Editing => Self::handle_text_input,
            Mode::Normal => Self::handle_normal_input,
        };
        if self.args.enable_module {
            if event::poll(std::time::Duration::from_millis(
                self.potato_module.update_time_ms(),
            ))? {
                return input_handler(self);
            }
        } else {
            return input_handler(self);
        }
        Ok(HandlerOperation::Nothing)
    }

    #[inline]
    fn write(&mut self) -> io::Result<()> {
        self.todo_app.write()
    }

    #[inline]
    fn handle_normal_input(&mut self) -> io::Result<HandlerOperation> {
        let event = event::read()?;
        if let Key(key) = event {
            let action = self.normal_keymaps.get_action(key);
            if let Some(action) = action {
                return Ok(action(self));
            }
        }
        Ok(HandlerOperation::Nothing)
    }

    #[inline]
    fn is_dependency_enabled(&self, todo: Option<&Todo>) -> bool {
        todo.map_or(false, |todo| {
            self.show_right && todo.dependency.is_some() && self.todo_app.is_tree()
        })
    }

    #[inline]
    fn render_module_widget(
        &self,
        frame: &mut Frame,
        direction: Direction,
        constraint1: Constraint,
        constraint2: Constraint,
    ) -> Rc<[Rect]> {
        let main_layout = Layout::default()
            .direction(direction)
            .constraints([constraint1, constraint2])
            .split(frame.size());
        frame.render_widget(self.potato_module.get_widget(), main_layout[0]);
        main_layout
    }

    #[inline]
    fn highlight_string(&self) -> &str {
        self.args.highlight_string.as_str()
    }

    #[inline]
    fn render_dependency_widget(
        &self,
        frame: &mut Frame,
        todo: Option<&Todo>,
        dependency_layout: Rect,
    ) {
        if let Some(todo) = todo {
            if let Some(note) = todo.dependency.as_ref().and_then(|dep| dep.note()) {
                match self.args.no_glow.not().then(|| {
                    let mut glow = Command::new("glow");
                    glow.stdin(Stdio::piped());
                    glow.stdout(Stdio::piped());
                    glow.spawn()
                }) {
                    Some(Ok(mut ps)) => {
                        ps.stdin.as_mut().map(|stdin| stdin.write(note.as_bytes()));
                        if let Ok(output) = ps.wait_with_output().map(|out| out.stdout) {
                            let note_widget = Paragraph::new(
                                str::from_utf8(&output)
                                    .unwrap_or("")
                                    .lines()
                                    .map(|line| line.trim_end())
                                    .map(|s| format!("{s}\n"))
                                    .collect::<String>(),
                            )
                            .wrap(Wrap { trim: false })
                            .block(default_block("Todo note"));
                            frame.render_widget(note_widget, dependency_layout);
                        }
                    }
                    _ => {
                        let note_widget = Paragraph::new(note)
                            .wrap(Wrap { trim: false })
                            .block(default_block("Todo note"));
                        frame.render_widget(note_widget, dependency_layout);
                    }
                }
            }
            if let Some(todo_list) = todo.dependency.as_ref().and_then(|dep| dep.todo_list()) {
                Self::render_todos_widget(
                    self.highlight_string(),
                    frame,
                    None,
                    dependency_layout,
                    self.todo_app.display_a_slice(
                        todo_list,
                        0,
                        dependency_layout.height as usize - 2,
                    ),
                    String::from("Todo dependencies"),
                )
            }
        }
    }

    #[inline(always)]
    fn render_current_todos_widget(
        &mut self,
        frame: &mut Frame,
        list_state: &mut ListState,
        todo_layout: Rect,
    ) {
        let title = self.title();
        let display = if self.args.minimal_render {
            let first = self.todo_app.index();
            let last = self
                .todo_app
                .current_list()
                .len(self.todo_app.get_restriction())
                .min(todo_layout.height as usize + first - 2);
            self.todo_app
                .display_a_slice(self.todo_app.current_list(), first, last)
        } else {
            self.todo_app.display_current_list()
        };
        Self::render_todos_widget(
            self.highlight_string(),
            frame,
            Some(list_state),
            todo_layout,
            display,
            title,
        )
    }

    #[inline(always)]
    fn render_todos_widget(
        highlight_symbol: &str,
        frame: &mut Frame,
        list_state: Option<&mut ListState>,
        todo_layout: Rect,
        display_list: Vec<String>,
        title: String,
    ) {
        match create_todo_widget(display_list, title, highlight_symbol) {
            TodoWidget::Paragraph(widget) => frame.render_widget(widget, todo_layout),
            TodoWidget::List(widget) => {
                if let Some(list_state) = list_state {
                    frame.render_stateful_widget(widget, todo_layout, list_state)
                } else {
                    frame.render_widget(widget, todo_layout)
                }
            }
        }
    }

    #[inline(always)]
    fn render_help_widget(&self, frame: &mut Frame) {
        let size = frame.size();
        let floating_window = Rect::new(
            size.width / 4,
            size.height / 4,
            size.width / 2,
            size.height / 2,
        );

        self.help_page.render(frame, floating_window);
    }

    #[inline]
    pub fn ui(&mut self, frame: &mut Frame, list_state: &mut ListState) {
        let todo = self.todo_app.todo();
        if !self.args.minimal_render {
            list_state.select(Some(self.todo_app.index()));
        }

        let dependency_enabled = self.is_dependency_enabled(todo);
        let dependency_width = if dependency_enabled { 40 } else { 0 };

        let main_layout = if self.args.enable_module {
            self.render_module_widget(
                frame,
                Direction::Vertical,
                Constraint::Length(5),
                Constraint::Min(0),
            )
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0)])
                .split(frame.size())
        };

        let todo_app_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(100 - dependency_width),
                Constraint::Percentage(dependency_width),
            ])
            .split(main_layout[self.args.enable_module as usize]);
        let is_editing = self.mode == Mode::Editing;

        let todo_and_textarea_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3 * is_editing as u16),
                Constraint::Min(0),
            ])
            .split(todo_app_layout[0]);
        if dependency_enabled {
            self.render_dependency_widget(frame, todo, todo_app_layout[1]);
        }

        if is_editing {
            frame.render_widget(self.textarea.widget(), todo_and_textarea_layout[0]);
        }
        self.render_current_todos_widget(frame, list_state, todo_and_textarea_layout[1]);
        if self.show_help {
            self.render_help_widget(frame);
        }
    }
}

pub fn default_block<'a, T>(title: T) -> Block<'a>
where
    T: Into<Line<'a>>,
{
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
}

pub enum TodoWidget<'a> {
    List(ratatui::widgets::List<'a>),
    Paragraph(ratatui::widgets::Paragraph<'a>),
}

pub fn create_todo_widget(
    display_list: Vec<String>,
    title: String,
    highlight_symbol: &str,
) -> TodoWidget<'_> {
    if display_list.is_empty() {
        TodoWidget::Paragraph(Paragraph::new("No todo.").block(default_block(title)))
    } else {
        TodoWidget::List(
            List::new(display_list)
                .block(default_block(title))
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
                .highlight_symbol(highlight_symbol)
                .repeat_highlight_symbol(true),
        )
    }
}

pub fn shutdown() -> io::Result<()> {
    terminal::disable_raw_mode()?;
    io::stdout()
        .execute(LeaveAlternateScreen)?
        .execute(crossterm::cursor::Show)?;
    let _ = io::stdout().flush();
    Ok(())
}

pub fn startup() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    io::stdout()
        .execute(EnterAlternateScreen)?
        .execute(crossterm::cursor::Hide)?;
    Ok(())
}

#[inline]
pub fn run(app: &mut App, args: TuiArgs) -> io::Result<()> {
    startup()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let mut list_state = ListState::default().with_selected(Some(0));
    let mut app = TuiApp::new(app, args);

    loop {
        terminal.draw(|frame| app.ui(frame, &mut list_state))?;

        let operation = app.handle_key_and_return_operation()?;
        match operation {
            HandlerOperation::Restart => {
                startup()?;
                terminal.swap_buffers();
            }
            HandlerOperation::Nothing => {}
        }
    }
}
