use async_trait::async_trait;

use crate::metadata::ReportMetadata;

use super::Reporter;

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
        let errs = futures::future::join_all(
            self.reporters
                .iter()
                .map(|reporter| reporter.report(jfr.clone(), metadata)),
        )
        .await;
        // return the first error
        errs.into_iter().collect()
    }
}

#[cfg(test)]
pub mod test {
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
        #[error("failed")]
        Failed,
    }

    #[derive(Debug)]
    struct ErrReporter;
    #[async_trait]
    impl Reporter for ErrReporter {
        async fn report(
            &self,
            _jfr: Vec<u8>,
            _metadata: &ReportMetadata,
        ) -> Result<(), Box<dyn std::error::Error + Send>> {
            Err(Box::new(Error::Failed))
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
            Box::new(ErrReporter) as Box<dyn Reporter + Send + Sync>,
            Box::new(OkReporter(signal_after.clone())) as Box<dyn Reporter + Send + Sync>,
        ]);
        // test that reports are done and return an error
        reporter.report(vec![], &DUMMY_METADATA).await.unwrap_err();
        // test that reports are done even though a reporter errored
        assert!(signal_before.load(atomic::Ordering::Relaxed));
        assert!(signal_after.load(atomic::Ordering::Relaxed));
    }
}
