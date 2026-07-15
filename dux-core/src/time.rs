use std::time::{SystemTime, UNIX_EPOCH};

/// Keep only values supported by Serde's `SystemTime` representation.
///
/// Filesystems can expose timestamps before the Unix epoch, but Serde rejects
/// those values. Treating them as unknown keeps cache writes available and lets
/// future actionable age guards fail closed instead of trusting a lossy value.
pub(crate) fn cache_serializable_time(time: SystemTime) -> Option<SystemTime> {
    time.duration_since(UNIX_EPOCH).is_ok().then_some(time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn cache_serializable_time_rejects_pre_epoch_values() {
        let before_epoch = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();

        assert_eq!(cache_serializable_time(before_epoch), None);
        assert_eq!(cache_serializable_time(UNIX_EPOCH), Some(UNIX_EPOCH));
    }
}
