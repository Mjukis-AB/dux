/// User actions that can be performed in the app
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Move selection up
    MoveUp,
    /// Move selection down
    MoveDown,
    /// Move selection up by a page
    PageUp,
    /// Move selection down by a page
    PageDown,
    /// Go to first item
    GoToFirst,
    /// Go to last item
    GoToLast,
    /// Expand selected directory
    Expand,
    /// Collapse selected directory
    Collapse,
    /// Toggle expand/collapse
    Toggle,
    /// Drill down into selected directory
    DrillDown,
    /// Go back to parent
    GoBack,
    /// Show help overlay
    ShowHelp,
    /// Hide help overlay
    HideHelp,
    /// Open selected item in Finder
    OpenInFinder,
    /// Request delete (show confirmation dialog)
    Delete,
    /// Confirm delete operation
    ConfirmDelete,
    /// Cancel delete operation
    CancelDelete,
    /// Quit the application
    Quit,
    /// Switch to next view
    NextView,
    /// Switch to previous view
    PrevView,
    /// Cycle stale threshold (Build Artifacts view)
    CycleStaleThreshold,
    /// Extend selection upward
    SelectUp,
    /// Extend selection downward
    SelectDown,
    /// Extend selection up by a page
    SelectPageUp,
    /// Extend selection down by a page
    SelectPageDown,
    /// Extend selection to first item
    SelectToFirst,
    /// Extend selection to last item
    SelectToLast,
    /// Toggle current item in/out of selection
    ToggleSelect,
    /// Clear multi-selection
    ClearSelection,
    /// Confirm multi-delete operation
    ConfirmMultiDelete,
    /// Cancel multi-delete operation
    CancelMultiDelete,
    /// No action (for tick events)
    Tick,
}
