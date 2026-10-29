use balthasar_model::Timestamp;

/// `YYYY-MM-DD` of a moment, in UTC.
pub(crate) fn day(at: Timestamp) -> String {
    balthasar_model::offered::day(at)
}
