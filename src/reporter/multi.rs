//! A reporter that reports profiling results to several destinations.

use async_trait::async_trait;

use crate::metadata::ReportMetadata;

use super::Reporter;

use std::fmt;

/// An aggregated error that contains an error per reporter. A reporter is identified
/// by the result of its Debug impl.
#[derive(Debug, thiserror::Error)]
struct MultiError {
    errors: Vec<(String, Box<dyn std::error::Error + Send>)>,
}

impl fmt::Display for MultiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{")?;
        let mut first = true;
        for (reporter, err) in self.errors.iter() {
            if !first {
                write!(f, ", ")?;
            }
            first = false;
            write!(f, "{}: {}", reporter, err)?;
        }
        write!(f, "}}")
    }
}

#[derive(Debug)]
/// A reporter that reports profiling results to several destinations.
///
/// If one of the destinations errors, it will continue reporting to the other ones.
pub struct MultiReporter {
    reporters: Vec<Box<dyn Reporter + Send + Sync>>,
}

impl MultiReporter {
    /// Create a new MultiReporter from a set of reporters
    pub fn new(reporters: Vec<Box<dyn Reporter + Send + Sync>>) -> Self {
        MultiReporter { reporters }
    }
}

#[async_trait]
impl Reporter for MultiReporter {
    async fn report(
        &self,
        jfr: Vec<u8>,
        metadata: &ReportMetadata,
    ) -> Result<(), Box<dyn std::error::Error + Send>> {
        let jfr_ref = &jfr[..];
        let errors = futures::future::join_all(self.reporters.iter().map(|reporter| async move {
            reporter
                .report(jfr_ref.to_owned(), metadata)
                .await
                .map_err(move |e| (format!("{:?}", reporter), e))
        }))
        .await;
        // return all errors
        let errors: Vec<_> = errors.into_iter().flat_map(|e| e.err()).collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Box::new(MultiError { errors }))
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        sync::{
            atomic::{self, AtomicBool},
            Arc,
        },
        time::Duration,
    };

    use async_trait::async_trait;

    use crate::{
        metadata::{ReportMetadata, DUMMY_METADATA},
        reporter::Reporter,
    };

    use super::MultiReporter;

    #[derive(Debug)]
    struct OkReporter(Arc<AtomicBool>);
    #[async_trait]
    impl Reporter for OkReporter {
        async fn report(
            &self,
            _jfr: Vec<u8>,
            _metadata: &ReportMetadata,
        ) -> Result<(), Box<dyn std::error::Error + Send>> {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            self.0.store(true, atomic::Ordering::Relaxed);
            Ok(())
        }
    }

    #[derive(Debug, thiserror::Error)]
    enum Error {
        #[error("failed: {0}")]
        Failed(String),
    }

    #[derive(Debug)]
    struct ErrReporter(String);
    #[async_trait]
    impl Reporter for ErrReporter {
        async fn report(
            &self,
            _jfr: Vec<u8>,
            _metadata: &ReportMetadata,
        ) -> Result<(), Box<dyn std::error::Error + Send>> {
            Err(Box::new(Error::Failed(self.0.clone())))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_multi_reporter_ok() {
        let signals: Vec<_> = (0..10).map(|_| Arc::new(AtomicBool::new(false))).collect();
        let reporter = MultiReporter::new(
            signals
                .iter()
                .map(|signal| {
                    Box::new(OkReporter(signal.clone())) as Box<dyn Reporter + Send + Sync>
                })
                .collect(),
        );
        // test that reports are done in parallel
        tokio::time::timeout(
            Duration::from_secs(2),
            reporter.report(vec![], &DUMMY_METADATA),
        )
        .await
        .unwrap()
        .unwrap();
        // test that reports are done
        assert!(signals.iter().all(|s| s.load(atomic::Ordering::Relaxed)));
    }

    #[tokio::test(start_paused = true)]
    async fn test_multi_reporter_err() {
        let signal_before = Arc::new(AtomicBool::new(false));
        let signal_after = Arc::new(AtomicBool::new(false));
        let reporter = MultiReporter::new(vec![
            Box::new(OkReporter(signal_before.clone())) as Box<dyn Reporter + Send + Sync>,
            Box::new(ErrReporter("foo".to_owned())) as Box<dyn Reporter + Send + Sync>,
            Box::new(ErrReporter("bar".to_owned())) as Box<dyn Reporter + Send + Sync>,
            Box::new(OkReporter(signal_after.clone())) as Box<dyn Reporter + Send + Sync>,
        ]);
        // test that reports are done and return an error
        let err = format!(
            "{}",
            reporter.report(vec![], &DUMMY_METADATA).await.unwrap_err()
        );
        assert_eq!(
            err,
            "{ErrReporter(\"foo\"): failed: foo, ErrReporter(\"bar\"): failed: bar}"
        );
        // test that reports are done even though a reporter errored
        assert!(signal_before.load(atomic::Ordering::Relaxed));
        assert!(signal_after.load(atomic::Ordering::Relaxed));
    }
}
