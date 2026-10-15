use std::sync::atomic::{AtomicUsize, Ordering};

/// JavaScript 安全整数上限（`Number.MAX_SAFE_INTEGER` = $2^{53} - 1 = 9\_007\_199\_254\_740\_991$）。
pub const JS_MAX_SAFE_INTEGER: usize = 9_007_199_254_740_991;

/// 生成在 JavaScript `number`（IEEE 754 双精度浮点数）安全整数范围内自增的 `usize`；
/// 当计数到达 [`JS_MAX_SAFE_INTEGER`] 极限时自动从 `0` 重新开始循环。
#[inline(always)]
pub fn next_safe_usize(counter: &AtomicUsize) -> usize {
    loop {
        let current = counter.load(Ordering::Relaxed);
        let next = if current >= JS_MAX_SAFE_INTEGER {
            0
        } else {
            current + 1
        };
        if counter
            .compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return current;
        }
    }
}

/// 线程安全的 JS 安全范围 `usize` 自增计数器（到达 [`JS_MAX_SAFE_INTEGER`] 自动归零）。
#[derive(Debug)]
pub struct SafeUsizeCounter {
    inner: AtomicUsize,
}

impl Default for SafeUsizeCounter {
    #[inline(always)]
    fn default() -> Self {
        Self::new(0)
    }
}

impl SafeUsizeCounter {
    #[inline(always)]
    pub const fn new(initial: usize) -> Self {
        Self {
            inner: AtomicUsize::new(initial),
        }
    }

    #[inline(always)]
    pub fn next(&self) -> usize {
        next_safe_usize(&self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_usize_counter_wraps_to_zero_at_max_safe_integer() {
        let counter = SafeUsizeCounter::new(JS_MAX_SAFE_INTEGER - 1);
        assert_eq!(counter.next(), JS_MAX_SAFE_INTEGER - 1);
        assert_eq!(counter.next(), JS_MAX_SAFE_INTEGER);
        assert_eq!(counter.next(), 0);
        assert_eq!(counter.next(), 1);
    }
}
