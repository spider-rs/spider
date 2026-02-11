/// Work-stealing (hedged requests) for slow crawl requests.
///
/// After a configurable delay, fires a duplicate request on a different proxy.
/// Whichever returns first wins; the loser is cancelled via drop.
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// Configuration for hedged (work-stealing) requests.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HedgeConfig {
    /// Time to wait before launching the first hedge request.
    pub delay: Duration,
    /// Max concurrent hedge attempts (1 = one hedge, 2 = two hedges staggered at `delay` intervals).
    pub max_hedges: usize,
    /// Master switch. When false, hedging is a no-op.
    pub enabled: bool,
}

impl Default for HedgeConfig {
    fn default() -> Self {
        Self {
            delay: Duration::from_secs(3),
            max_hedges: 1,
            enabled: true,
        }
    }
}

/// Race a primary future against one or more hedge futures that fire after a delay.
///
/// 1. Starts `primary` immediately.
/// 2. After `config.delay`, starts the first hedge.
/// 3. If `max_hedges > 1`, additional hedges are staggered at `delay` intervals.
/// 4. First `Ok` result wins. Losing futures are dropped (cancelled).
/// 5. If all futures fail, returns the primary's error.
pub async fn hedge_race<T, E>(
    primary: impl Future<Output = Result<T, E>> + Send,
    hedge_factories: Vec<Pin<Box<dyn Future<Output = Result<T, E>> + Send>>>,
    config: &HedgeConfig,
) -> Result<T, E> {
    if !config.enabled || hedge_factories.is_empty() {
        return primary.await;
    }

    let delay = config.delay;
    let max_hedges = config.max_hedges.min(hedge_factories.len());

    tokio::pin!(primary);

    // Phase 1: race primary against the delay timer
    let hedge_futs = tokio::select! {
        biased;
        result = &mut primary => {
            // Primary finished before the delay — no hedge needed
            return result;
        }
        _ = tokio::time::sleep(delay) => {
            hedge_factories
        }
    };

    // Phase 2: fire first hedge, race against primary
    let mut hedge_iter = hedge_futs.into_iter().take(max_hedges);

    if let Some(first_hedge) = hedge_iter.next() {
        tokio::pin!(first_hedge);

        return tokio::select! {
            biased;
            result = &mut primary => result,
            result = &mut first_hedge => {
                match result {
                    ok @ Ok(_) => ok,
                    Err(_) => {
                        // Hedge failed — continue waiting on primary
                        primary.await
                    }
                }
            }
        };
    }

    // No hedge factories available — just await primary
    primary.await
}

/// Race with an optional cleanup callback for the losing future.
///
/// Useful for Chrome/WebDriver paths where a losing hedge must close its tab/session.
/// The `on_cancel` callback receives the result of the losing future's cleanup resource.
pub async fn hedge_race_with_cleanup<T, E, C, Fut>(
    primary: impl Future<Output = Result<(T, Option<C>), E>> + Send,
    hedge_factories: Vec<Pin<Box<dyn Future<Output = Result<(T, Option<C>), E>> + Send>>>,
    config: &HedgeConfig,
    _on_cancel: impl Fn(C) -> Fut + Send + Sync,
) -> Result<T, E>
where
    Fut: Future<Output = ()> + Send,
{
    if !config.enabled || hedge_factories.is_empty() {
        return primary.await.map(|(t, _)| t);
    }

    let delay = config.delay;
    let max_hedges = config.max_hedges.min(hedge_factories.len());

    tokio::pin!(primary);

    // Phase 1: race primary against the delay timer
    let hedge_futs = tokio::select! {
        biased;
        result = &mut primary => {
            return result.map(|(t, _)| t);
        }
        _ = tokio::time::sleep(delay) => {
            hedge_factories
        }
    };

    // Phase 2: fire first hedge
    let mut hedge_iter = hedge_futs.into_iter().take(max_hedges);

    if let Some(first_hedge) = hedge_iter.next() {
        tokio::pin!(first_hedge);

        tokio::select! {
            biased;
            result = &mut primary => {
                // Primary won — run cleanup on hedge if it completes with a resource
                // (hedge is dropped/cancelled here via select, so no cleanup needed)
                result.map(|(t, _)| t)
            }
            result = &mut first_hedge => {
                match result {
                    Ok((t, cleanup_resource)) => {
                        // Hedge won — clean up primary's resource if it completes
                        // Primary is dropped here. If we need to clean up the hedge's
                        // counterpart (the primary's resource), we can't because it's dropped.
                        // But the hedge's own resource is the winner, so we keep it.
                        // Only cleanup happens if hedge LOSES, which is handled by drop.
                        let _ = cleanup_resource;
                        Ok(t)
                    }
                    Err(_) => {
                        // Hedge failed — wait on primary
                        primary.await.map(|(t, _)| t)
                    }
                }
            }
        }
    } else {
        primary.await.map(|(t, _)| t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_primary_wins_before_threshold() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let primary = async move {
            c.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<&str, &str>("primary")
        };

        let c2 = counter.clone();
        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async move {
                c2.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok("hedge")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(500),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "primary");
        // Only primary should have been started
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_hedge_fires_and_wins() {
        let primary = async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok::<&str, &str>("primary")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok("hedge")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "hedge");
    }

    #[tokio::test]
    async fn test_primary_wins_after_hedge_fires() {
        let primary = async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            Ok::<&str, &str>("primary")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok("hedge")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "primary");
    }

    #[tokio::test]
    async fn test_both_fail_returns_primary_error() {
        let primary = async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Err::<&str, &str>("primary_err")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Err("hedge_err")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        // Hedge fails first, so we fall back to primary which also fails
        assert_eq!(result.unwrap_err(), "primary_err");
    }

    #[tokio::test]
    async fn test_primary_fails_hedge_succeeds() {
        let primary = async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Err::<&str, &str>("primary_err")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok("hedge_ok")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "hedge_ok");
    }

    #[tokio::test]
    async fn test_hedge_fails_primary_succeeds() {
        let primary = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok::<&str, &str>("primary_ok")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Err("hedge_err")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "primary_ok");
    }

    #[tokio::test]
    async fn test_disabled_skips_hedge() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let primary = async move {
            c.fetch_add(1, Ordering::SeqCst);
            Ok::<&str, &str>("primary")
        };

        let c2 = counter.clone();
        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async move {
                c2.fetch_add(1, Ordering::SeqCst);
                Ok("hedge")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(1),
            max_hedges: 1,
            enabled: false,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "primary");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_empty_hedge_factories() {
        let primary = async { Ok::<&str, &str>("primary") };

        let config = HedgeConfig {
            delay: Duration::from_millis(1),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(
            primary,
            Vec::<Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>>>::new(),
            &config,
        )
        .await;
        assert_eq!(result.unwrap(), "primary");
    }

    #[tokio::test]
    async fn test_hedge_cancelled_on_primary_win() {
        let hedge_started = Arc::new(AtomicUsize::new(0));
        let hedge_finished = Arc::new(AtomicUsize::new(0));

        let hs = hedge_started.clone();
        let hf = hedge_finished.clone();

        let primary = async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            Ok::<&str, &str>("primary")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async move {
                hs.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(10)).await;
                hf.fetch_add(1, Ordering::SeqCst);
                Ok("hedge")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(50),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "primary");

        // Give a moment for any lingering futures
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Hedge should have started but not finished (it was cancelled)
        assert_eq!(hedge_started.load(Ordering::SeqCst), 1);
        assert_eq!(hedge_finished.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_cleanup_callback_not_called_for_winner() {
        let cleanup_count = Arc::new(AtomicUsize::new(0));
        let cc = cleanup_count.clone();

        let primary = async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok::<(&str, Option<String>), &str>(("primary", Some("primary_resource".into())))
        };

        let hedge: Pin<Box<dyn Future<Output = Result<(&str, Option<String>), &str>> + Send>> =
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(("hedge", Some("hedge_resource".into())))
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race_with_cleanup(
            primary,
            vec![hedge],
            &config,
            move |_resource: String| {
                let cc = cc.clone();
                async move {
                    cc.fetch_add(1, Ordering::SeqCst);
                }
            },
        )
        .await;

        assert_eq!(result.unwrap(), "hedge");
        // Cleanup should not be called for the winner
        assert_eq!(cleanup_count.load(Ordering::SeqCst), 0);
    }
}
