use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{Action, AppMode};

/// Map key events to actions based on current mode
pub fn handle_key(key: KeyEvent, mode: AppMode) -> Action {
    match mode {
        AppMode::Help => handle_key_help(key),
        AppMode::Scanning | AppMode::Finalizing => handle_key_scanning(key),
        AppMode::Browsing => handle_key_browsing(key),
        AppMode::ConfirmDelete => handle_key_confirm_delete(key),
    }
}

fn handle_key_help(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => Action::HideHelp,
        _ => Action::Tick,
    }
}

fn handle_key_scanning(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        _ => Action::Tick,
    }
}

fn handle_key_browsing(key: KeyEvent) -> Action {
    match key.code {
        // Quit
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,

        // Navigation
        KeyCode::Up | KeyCode::Char('k') => Action::MoveUp,
        KeyCode::Down | KeyCode::Char('j') => Action::MoveDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::Home | KeyCode::Char('g') => Action::GoToFirst,
        KeyCode::End | KeyCode::Char('G') => Action::GoToLast,

        // Expand/Collapse
        KeyCode::Right | KeyCode::Char('l') => Action::Expand,
        KeyCode::Left | KeyCode::Char('h') => Action::Collapse,
        KeyCode::Char(' ') | KeyCode::Tab => Action::Toggle,

        // Drill down / back
        KeyCode::Enter => Action::DrillDown,
        KeyCode::Backspace | KeyCode::Esc => Action::GoBack,

        // Help
        KeyCode::Char('?') => Action::ShowHelp,

        // Open in Finder
        KeyCode::Char('o') => Action::OpenInFinder,

        // Delete
        KeyCode::Char('d') => Action::Delete,

        _ => Action::Tick,
    }
}

fn handle_key_confirm_delete(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => Action::ConfirmDelete,
        KeyCode::Char('n') | KeyCode::Esc => Action::CancelDelete,
        _ => Action::Tick,
    }
}
