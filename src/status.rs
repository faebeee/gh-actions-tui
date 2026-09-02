//! Categorizes a workflow run's raw GitHub `status`/`conclusion` strings
//! into a small set of outcomes.
//!
//! Both the status filter (`app::StatusFilter`) and the UI's status color
//! (`ui::status_style`) need to bucket the same raw API strings into
//! logical categories. Having a single classification function here keeps
//! those two call sites from silently drifting apart as GitHub adds new
//! status/conclusion values.

/// A simplified bucket for a run's status/conclusion, independent of the
/// specific GitHub API string values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Success,
    Failure,
    Cancelled,
    Skipped,
    InProgress,
    Completed,
    Unknown,
}

impl RunOutcome {
    /// Classifies a run's raw `status` and optional `conclusion` fields (as
    /// returned by the GitHub API) into a [`RunOutcome`].
    pub fn classify(status: &str, conclusion: Option<&str>) -> Self {
        if let Some(conclusion) = conclusion {
            match conclusion {
                "success" => RunOutcome::Success,
                "failure" | "timed_out" | "action_required" => RunOutcome::Failure,
                "cancelled" => RunOutcome::Cancelled,
                "skipped" | "neutral" => RunOutcome::Skipped,
                _ => RunOutcome::Unknown,
            }
        } else {
            match status {
                "in_progress" | "queued" | "waiting" | "pending" => RunOutcome::InProgress,
                "completed" => RunOutcome::Completed,
                _ => RunOutcome::Unknown,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_success() {
        assert_eq!(
            RunOutcome::classify("completed", Some("success")),
            RunOutcome::Success
        );
    }

    #[test]
    fn classifies_failure_variants() {
        for conclusion in ["failure", "timed_out", "action_required"] {
            assert_eq!(
                RunOutcome::classify("completed", Some(conclusion)),
                RunOutcome::Failure,
                "conclusion {conclusion} should classify as Failure"
            );
        }
    }

    #[test]
    fn classifies_in_progress_variants() {
        for status in ["in_progress", "queued", "waiting", "pending"] {
            assert_eq!(
                RunOutcome::classify(status, None),
                RunOutcome::InProgress,
                "status {status} should classify as InProgress"
            );
        }
    }

    #[test]
    fn classifies_cancelled_and_skipped() {
        assert_eq!(
            RunOutcome::classify("completed", Some("cancelled")),
            RunOutcome::Cancelled
        );
        assert_eq!(
            RunOutcome::classify("completed", Some("skipped")),
            RunOutcome::Skipped
        );
        assert_eq!(
            RunOutcome::classify("completed", Some("neutral")),
            RunOutcome::Skipped
        );
    }

    #[test]
    fn classifies_completed_with_no_conclusion() {
        assert_eq!(
            RunOutcome::classify("completed", None),
            RunOutcome::Completed
        );
    }

    #[test]
    fn classifies_unknown_fallback() {
        assert_eq!(RunOutcome::classify("weird_status", None), RunOutcome::Unknown);
        assert_eq!(
            RunOutcome::classify("completed", Some("weird_conclusion")),
            RunOutcome::Unknown
        );
    }
}
