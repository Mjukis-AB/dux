/// Observed disk-pressure classification for a volume.
///
/// This is presentation and scheduling policy, not cleanup authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskPressure {
    Healthy,
    Warning,
    Critical,
    Unknown,
}
