/// A reset-shutdown capability was consumed without proving worker
/// quiescence. The old engine remains terminal and the reset effect must not
/// run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppDataResetShutdownError {
    #[error("engine workers did not quiesce within the bounded reset deadline")]
    ShutdownIncomplete,
    #[error("the reset terminal lifecycle claim is internally inconsistent")]
    InternalState,
}
