use regex::Regex;
use std::sync::LazyLock;

/// Key in the elision marker: number of elements dropped.
pub const ELIDED_KEY: &str = "__retoken_elided__";

/// Key for human-readable note on the first elision marker of a payload.
pub const ELIDED_NOTE_KEY: &str = "__retoken_note__";

/// Matches object keys whose subtrees carry error/warning signal and must be
/// preserved verbatim. Ported from legacy `errorKeyRe`.
pub static ERROR_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(error|errors|message|msg|stack|stacktrace|stack_trace|trace|traceback|exception|reason|detail|details|warning|warnings)$")
        .expect("ERROR_KEY_RE must compile")
});

/// Matches critical keywords anywhere in an element's canonical JSON, so items
/// in an error/failure state are force-kept regardless of position. Ported from
/// legacy `errorValueRe` and extended.
pub static ERROR_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(error|errors|exception|failed|failure|critical|fatal|crash|panic|abort|timeout|denied|rejected|refused|broken|segfault)\b")
        .expect("ERROR_VALUE_RE must compile")
});
