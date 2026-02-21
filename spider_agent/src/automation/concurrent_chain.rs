//! Concurrent action chain execution with dependency management.
//!
//! This module provides the [`execute_graph`] function for executing actions
//! in parallel where dependencies allow. Independent actions run concurrently
//! via `tokio::JoinSet`, while dependent actions respect their ordering
//! constraints.
//!
//! Data types ([`DependentStep`], [`DependencyGraph`], [`ConcurrentChainConfig`],
//! [`ConcurrentChainResult`], [`StepResult`]) live in [`spider_agent_types`]
//! and are re-exported at the parent module level.

use super::{
    ConcurrentChainConfig, ConcurrentChainResult, DependencyGraph, DependentStep, StepResult,
};
use std::time::Instant;

/// Execute a dependency graph using a provided executor function.
///
/// This manages the execution loop for a dependency graph, calling the
/// executor for each step and managing parallelism via `tokio::JoinSet`.
///
/// # Arguments
/// * `graph` - The dependency graph to execute (mutated as steps complete)
/// * `config` - Execution configuration (max parallelism, failure handling)
/// * `executor` - Async function that executes a single step
///
/// # Returns
/// A [`ConcurrentChainResult`] summarizing the execution outcome.
pub async fn execute_graph<F, Fut>(
    graph: &mut DependencyGraph,
    config: &ConcurrentChainConfig,
    executor: F,
) -> ConcurrentChainResult
where
    F: Fn(DependentStep) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = StepResult> + Send + 'static,
{
    let start = Instant::now();
    let mut result = ConcurrentChainResult::new();
    let mut max_parallel_seen = 0;

    while !graph.is_complete() {
        if config.stop_on_failure && graph.has_failed() {
            break;
        }

        let ready_ids = graph.take_ready_steps(config.max_parallel);
        if ready_ids.is_empty() {
            // No ready steps but not complete - something is wrong
            break;
        }

        max_parallel_seen = max_parallel_seen.max(ready_ids.len());

        if config.enable_parallel && ready_ids.len() > 1 {
            // Execute in parallel using JoinSet
            let mut join_set = tokio::task::JoinSet::new();

            for step_id in ready_ids {
                if let Some(step) = graph.get_step(&step_id).cloned() {
                    let exec = executor.clone();
                    let id = step_id.clone();
                    join_set.spawn(async move {
                        let step_start = Instant::now();
                        let mut step_result = exec(step).await;
                        step_result.duration_ms = step_start.elapsed().as_millis() as u64;
                        (id, step_result)
                    });
                }
            }

            // Collect results
            while let Some(res) = join_set.join_next().await {
                match res {
                    Ok((id, step_result)) => {
                        graph.complete(&id, step_result.clone());
                        result.record(id, step_result);
                    }
                    Err(e) => {
                        // Task panicked
                        let err_result = StepResult::failure(format!("Task panic: {}", e));
                        result.record("unknown".to_string(), err_result);
                    }
                }
            }
        } else {
            // Execute sequentially
            for step_id in ready_ids {
                if let Some(step) = graph.get_step(&step_id).cloned() {
                    let step_start = Instant::now();
                    let mut step_result = executor(step).await;
                    step_result.duration_ms = step_start.elapsed().as_millis() as u64;
                    graph.complete(&step_id, step_result.clone());
                    result.record(step_id, step_result);

                    if config.stop_on_failure && graph.has_failed() {
                        break;
                    }
                }
            }
        }
    }

    result.max_parallel = max_parallel_seen;
    result.duration_ms = start.elapsed().as_millis() as u64;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execute_graph() {
        let steps = vec![
            DependentStep::new("a", serde_json::json!({"Wait": 10})),
            DependentStep::new("b", serde_json::json!({"Wait": 10})),
            DependentStep::new("c", serde_json::json!({"Wait": 10})).depends_on("a"),
        ];

        let mut graph = DependencyGraph::new(steps).expect("valid graph");
        let config = ConcurrentChainConfig::new().with_max_parallel(2);

        let result =
            execute_graph(&mut graph, &config, |_step| async { StepResult::success() }).await;

        assert!(result.success);
        assert_eq!(result.steps_executed, 3);
        assert!(result.max_parallel >= 1);
    }
}
