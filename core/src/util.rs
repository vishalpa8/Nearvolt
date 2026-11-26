//! Small helpers shared across the engine.

use tokio::task::JoinHandle;

/// Aborts the task when dropped, so background work never outlives its owner.
pub struct AbortOnDrop<T>(pub JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
