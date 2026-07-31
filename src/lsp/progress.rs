//! Work-done progress reporting for the background index/sidecar jobs.
//!
//! The single-thread index pool runs the two unbounded-duration jobs
//! (`build_index` and the sidecar fetch); both were previously invisible. A
//! [`ProgressReporter`] brackets a job with LSP `$/progress` begin/report/end,
//! routed to the client through the existing outbound channel: it sends
//! [`Outbound::Progress`] on an `out_tx` clone, and the main loop forwards each
//! one to the client (gated on the client capability; see
//! [`GlobalState::on_progress`](super::GlobalState)). The reporter never touches
//! the client connection directly—it only owns an `out_tx` clone, exactly like
//! the diagnostics path.

use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

/// Monotonic source of unique progress tokens. A build and a sidecar fetch can be
/// queued back-to-back on the pool, so each operation needs its own token.
static PROGRESS_SEQ: AtomicU64 = AtomicU64::new(0);

/// A handle that brackets one background job with work-done progress. Created
/// with [`begin`](Self::begin) (emits `Begin`), updated with
/// [`report`](Self::report), and closed with [`end`](Self::end) or on drop—the
/// [`Drop`] impl guarantees an `End` even on an early return or a panic (the
/// index pool wraps each job in `catch_unwind`), so a progress bar never hangs.
pub(crate) struct ProgressReporter {
    out_tx: Sender<Outbound>,
    token: String,
    /// Set once an `End` has been emitted, so [`Drop`] doesn't emit a second one.
    ended: bool,
}

impl ProgressReporter {
    /// Begin a work-done progress operation with a fresh token, emitting `Begin`.
    /// `message` is the optional detail line shown under the `title` (e.g.
    /// "3 packages").
    pub(crate) fn begin(out_tx: Sender<Outbound>, title: &str, message: Option<String>) -> Self {
        let n = PROGRESS_SEQ.fetch_add(1, Ordering::Relaxed);
        let token = format!("arity/progress/{n}");
        let work = WorkDoneProgress::Begin(WorkDoneProgressBegin {
            title: title.to_string(),
            cancellable: Some(false),
            message,
            percentage: None,
        });
        let _ = out_tx.send(Outbound::Progress {
            token: token.clone(),
            work,
        });
        Self {
            out_tx,
            token,
            ended: false,
        }
    }

    /// Emit a `Report` update. `percentage` (0–100) is optional; omit it for
    /// indeterminate progress.
    pub(crate) fn report(&self, message: String, percentage: Option<u32>) {
        let work = WorkDoneProgress::Report(WorkDoneProgressReport {
            cancellable: Some(false),
            message: Some(message),
            percentage,
        });
        let _ = self.out_tx.send(Outbound::Progress {
            token: self.token.clone(),
            work,
        });
    }

    /// Close the operation with a final `End` (optionally a completion message).
    /// Consumes the reporter; the subsequent drop is a no-op (guarded by `ended`).
    pub(crate) fn end(mut self, message: Option<String>) {
        self.finish(message);
    }

    /// Emit `End` exactly once. Shared by [`end`](Self::end) and [`Drop`].
    fn finish(&mut self, message: Option<String>) {
        if self.ended {
            return;
        }
        self.ended = true;
        let work = WorkDoneProgress::End(WorkDoneProgressEnd { message });
        let _ = self.out_tx.send(Outbound::Progress {
            token: self.token.clone(),
            work,
        });
    }
}

impl Drop for ProgressReporter {
    fn drop(&mut self) {
        // Panic-safety: close the progress even if the job unwound or returned
        // early without calling `end`.
        self.finish(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pull the `(token, work)` out of an [`Outbound::Progress`], panicking on any
    /// other variant.
    fn progress(ob: Outbound) -> (String, WorkDoneProgress) {
        match ob {
            Outbound::Progress { token, work } => (token, work),
            _ => panic!("expected Outbound::Progress"),
        }
    }

    #[test]
    fn begin_report_end_emit_in_order_with_one_token() {
        let (tx, rx) = crossbeam_channel::unbounded::<Outbound>();
        let reporter = ProgressReporter::begin(tx, "Indexing", Some("2 packages".to_string()));
        reporter.report("magrittr".to_string(), Some(50));
        reporter.end(Some("done".to_string()));

        let (t0, w0) = progress(rx.recv().unwrap());
        let (t1, w1) = progress(rx.recv().unwrap());
        let (t2, w2) = progress(rx.recv().unwrap());
        // Exactly three messages, all sharing the one token.
        assert!(rx.try_recv().is_err(), "no extra messages after End");
        assert_eq!(t0, t1);
        assert_eq!(t1, t2);
        assert!(matches!(w0, WorkDoneProgress::Begin(_)));
        assert!(matches!(w1, WorkDoneProgress::Report(_)));
        assert!(matches!(w2, WorkDoneProgress::End(_)));
    }

    #[test]
    fn drop_without_end_still_emits_exactly_one_end() {
        let (tx, rx) = crossbeam_channel::unbounded::<Outbound>();
        {
            let _reporter = ProgressReporter::begin(tx, "Fetching", None);
            // No explicit `end`: the drop at the end of this scope must close it.
        }
        let (_, begin) = progress(rx.recv().unwrap());
        assert!(matches!(begin, WorkDoneProgress::Begin(_)));
        let (_, end) = progress(rx.recv().unwrap());
        assert!(matches!(end, WorkDoneProgress::End(_)));
        assert!(rx.try_recv().is_err(), "drop emits End only once");
    }

    #[test]
    fn explicit_end_suppresses_drop_end() {
        let (tx, rx) = crossbeam_channel::unbounded::<Outbound>();
        let reporter = ProgressReporter::begin(tx, "Indexing", None);
        reporter.end(None); // consumes + drops
        let _ = progress(rx.recv().unwrap()); // Begin
        let (_, end) = progress(rx.recv().unwrap());
        assert!(matches!(end, WorkDoneProgress::End(_)));
        assert!(rx.try_recv().is_err(), "no double End from the drop");
    }

    #[test]
    fn distinct_reporters_get_distinct_tokens() {
        let (tx, rx) = crossbeam_channel::unbounded::<Outbound>();
        let a = ProgressReporter::begin(tx.clone(), "A", None);
        let b = ProgressReporter::begin(tx, "B", None);
        let (ta, _) = progress(rx.recv().unwrap());
        let (tb, _) = progress(rx.recv().unwrap());
        assert_ne!(ta, tb);
        drop(a);
        drop(b);
    }
}
