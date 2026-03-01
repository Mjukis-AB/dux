use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{Action, AppMode};

/// Map key events to actions based on current mode
pub fn handle_key(key: KeyEvent, mode: AppMode, has_selection: bool, selecting: bool) -> Action {
    match mode {
        AppMode::Help => handle_key_help(key),
        AppMode::Scanning | AppMode::Finalizing => handle_key_scanning(key),
        AppMode::Browsing => handle_key_browsing(key, has_selection, selecting),
        AppMode::ConfirmDelete => handle_key_confirm_delete(key),
        AppMode::ConfirmMultiDelete => handle_key_confirm_multi_delete(key),
        AppMode::MultiDeleting => handle_key_multi_deleting(key),
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

fn handle_key_browsing(key: KeyEvent, has_selection: bool, selecting: bool) -> Action {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT) || selecting;

    match key.code {
        // Quit
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,

        // Toggle selecting mode
        KeyCode::Char('v') => Action::ToggleSelect,

        // Shift/selecting-mode navigation = extend selection
        KeyCode::Up if shift => Action::SelectUp,
        KeyCode::Down if shift => Action::SelectDown,
        KeyCode::Char('K') => Action::SelectUp,
        KeyCode::Char('J') => Action::SelectDown,
        KeyCode::Char('k') if selecting => Action::SelectUp,
        KeyCode::Char('j') if selecting => Action::SelectDown,
        KeyCode::PageUp if shift => Action::SelectPageUp,
        KeyCode::PageDown if shift => Action::SelectPageDown,
        KeyCode::Home if shift => Action::SelectToFirst,
        KeyCode::End if shift => Action::SelectToLast,
        KeyCode::Char('g') if selecting => Action::SelectToFirst,
        KeyCode::Char('G') if selecting => Action::SelectToLast,

        // Normal navigation
        KeyCode::Up | KeyCode::Char('k') => Action::MoveUp,
        KeyCode::Down | KeyCode::Char('j') => Action::MoveDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::Home | KeyCode::Char('g') => Action::GoToFirst,
        KeyCode::End | KeyCode::Char('G') => Action::GoToLast,

        // Expand/Collapse
        KeyCode::Right | KeyCode::Char('l') => Action::Expand,
        KeyCode::Left | KeyCode::Char('h') => Action::Collapse,
        KeyCode::Char(' ') => Action::Toggle,

        // View switching
        KeyCode::Tab => Action::NextView,
        KeyCode::BackTab => Action::PrevView,

        // Stale threshold cycling
        KeyCode::Char('s') => Action::CycleStaleThreshold,

        // Drill down / back
        KeyCode::Enter => Action::DrillDown,
        KeyCode::Backspace => Action::GoBack,

        // Esc: clear selection first, then go back
        KeyCode::Esc => {
            if has_selection || selecting {
                Action::ClearSelection
            } else {
                Action::GoBack
            }
        }

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

fn handle_key_confirm_multi_delete(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => Action::ConfirmMultiDelete,
        KeyCode::Char('n') | KeyCode::Esc => Action::CancelMultiDelete,
        _ => Action::Tick,
    }
}

fn handle_key_multi_deleting(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        _ => Action::Tick,
    }
}
