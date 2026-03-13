//! Pre-allocated buffer pool for TFTP session packet buffers.
//!
//! Reduces allocator pressure by recycling `Vec<u8>` buffers between sessions.
//! Uses a simple `Mutex<Vec>` which is sufficient for typical concurrency (~100 sessions).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// A lock-based buffer pool for reusing packet buffers across sessions.
pub struct BufferPool {
    pool: Mutex<Vec<Vec<u8>>>,
    buf_size: usize,
    capacity: usize,
    pub hits: AtomicU64,
    pub misses: AtomicU64,
}

impl BufferPool {
    /// Create a new buffer pool.
    ///
    /// - `capacity`: maximum number of buffers to keep in the pool
    /// - `buf_size`: size of each buffer (typically `max_blksize + 4`)
    pub fn new(capacity: usize, buf_size: usize) -> Self {
        Self {
            pool: Mutex::new(Vec::with_capacity(capacity)),
            buf_size,
            capacity,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Acquire a buffer from the pool, or allocate a new one if empty.
    /// The returned buffer is zeroed and sized to `buf_size`.
    pub fn acquire(&self) -> Vec<u8> {
        let mut pool = self.pool.lock().unwrap();
        if let Some(mut buf) = pool.pop() {
            buf.clear();
            buf.resize(self.buf_size, 0);
            self.hits.fetch_add(1, Ordering::Relaxed);
            buf
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            vec![0u8; self.buf_size]
        }
    }

    /// Release a buffer back into the pool for reuse.
    /// If the pool is full, the buffer is dropped.
    pub fn release(&self, buf: Vec<u8>) {
        let mut pool = self.pool.lock().unwrap();
        if pool.len() < self.capacity {
            pool.push(buf);
        }
        // else: drop — pool is full
    }

    /// Current number of buffers in the pool.
    #[allow(dead_code)]
    pub fn available(&self) -> usize {
        self.pool.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acquire_release_cycle() {
        let pool = BufferPool::new(4, 1024);

        // First acquire — miss (pool empty)
        let buf1 = pool.acquire();
        assert_eq!(buf1.len(), 1024);
        assert_eq!(pool.misses.load(Ordering::Relaxed), 1);

        // Release and re-acquire — hit
        pool.release(buf1);
        assert_eq!(pool.available(), 1);

        let buf2 = pool.acquire();
        assert_eq!(buf2.len(), 1024);
        assert_eq!(pool.hits.load(Ordering::Relaxed), 1);
        assert_eq!(pool.available(), 0);
    }

    #[test]
    fn test_capacity_limit() {
        let pool = BufferPool::new(2, 512);
        let b1 = pool.acquire();
        let b2 = pool.acquire();
        let b3 = pool.acquire();

        pool.release(b1);
        pool.release(b2);
        pool.release(b3); // exceeds capacity — dropped

        assert_eq!(pool.available(), 2);
    }
}
