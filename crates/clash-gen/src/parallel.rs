//! [`RayonRunner`]: realising a batch of tracts on several threads.
//!
//! Realisation is pure, and [`realize_batch`](clash_world::realize_batch)
//! puts each result in a place of its own, so which thread does which
//! tract, and when, cannot change what comes out (01 §4, invariant W6).

use core::fmt;

use clash_world::BatchRunner;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use thiserror::Error;

/// Why a pool of threads could not be made.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("could not start a pool of {threads} threads: {message}")]
pub struct PoolError {
    /// How many threads were asked for.
    pub threads: usize,
    /// What the pool builder said.
    pub message: String,
}

/// Runs batches on a pool of threads.
pub struct RayonRunner {
    /// A pool of its own, or `None` for the process's shared pool.
    pool: Option<rayon::ThreadPool>,
}

impl RayonRunner {
    /// A runner on the process's shared pool, which has a thread for
    /// each processor unless the process set it up otherwise.
    pub fn global() -> RayonRunner {
        RayonRunner { pool: None }
    }

    /// A runner on a pool of its own with this many threads. Zero means
    /// a thread for each processor.
    pub fn with_threads(threads: usize) -> Result<RayonRunner, PoolError> {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .map(|pool| RayonRunner { pool: Some(pool) })
            .map_err(|error| PoolError {
                threads,
                message: error.to_string(),
            })
    }

    /// How many threads the runner's pool has.
    pub fn threads(&self) -> usize {
        match &self.pool {
            Some(pool) => pool.current_num_threads(),
            None => rayon::current_num_threads(),
        }
    }
}

impl fmt::Debug for RayonRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RayonRunner")
            .field("own_pool", &self.pool.is_some())
            .finish()
    }
}

impl BatchRunner for RayonRunner {
    fn run(&self, count: usize, job: &(dyn Fn(usize) + Sync)) {
        let all = || (0..count).into_par_iter().for_each(job);
        match &self.pool {
            Some(pool) => pool.install(all),
            None => all(),
        }
    }
}
