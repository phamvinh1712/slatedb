use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

const SHARD_COUNT: usize = 64;

#[global_allocator]
static ALLOCATOR: CountingAllocator<System> = CountingAllocator::new(System);

#[uniffi::export]
pub fn native_allocated_bytes() -> u64 {
    ALLOCATOR.live_bytes() as u64
}

struct CountingAllocator<A> {
    inner: A,
    shards: [AllocationShard; SHARD_COUNT],
}

#[repr(align(128))]
struct AllocationShard(AtomicUsize);

impl<A> CountingAllocator<A> {
    const fn new(inner: A) -> Self {
        Self {
            inner,
            shards: [const { AllocationShard(AtomicUsize::new(0)) }; SHARD_COUNT],
        }
    }

    fn live_bytes(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| shard.0.load(Ordering::Relaxed))
            .sum()
    }

    fn shard(&self, ptr: *mut u8) -> &AtomicUsize {
        let address = ptr.addr();
        let hash = (address >> 4) ^ (address >> 13) ^ (address >> 22) ^ (address >> 30);
        &self.shards[hash & (SHARD_COUNT - 1)].0
    }
}

unsafe impl<A: GlobalAlloc> GlobalAlloc for CountingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() {
            self.shard(ptr).fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.inner.alloc_zeroed(layout) };
        if !ptr.is_null() {
            self.shard(ptr).fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.shard(ptr).fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { self.inner.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let previous = self.shard(ptr);
        let resized = unsafe { self.inner.realloc(ptr, layout, new_size) };
        if !resized.is_null() {
            previous.fetch_sub(layout.size(), Ordering::Relaxed);
            self.shard(resized).fetch_add(new_size, Ordering::Relaxed);
        }
        resized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;
    use std::sync::atomic::AtomicPtr;
    use std::thread;

    #[test]
    fn counts_zeroed_allocation_and_resizes() {
        let allocator = CountingAllocator::new(System);
        let mut layout = Layout::from_size_align(64, 8).unwrap();

        unsafe {
            let mut ptr = allocator.alloc_zeroed(layout);
            assert!(!ptr.is_null());
            assert_eq!(allocator.live_bytes(), layout.size());
            assert_eq!(std::slice::from_raw_parts(ptr, layout.size()), &[0; 64]);

            for size in [256, 16] {
                ptr = allocator.realloc(ptr, layout, size);
                assert!(!ptr.is_null());
                layout = Layout::from_size_align(size, layout.align()).unwrap();
                assert_eq!(allocator.live_bytes(), size);
            }

            allocator.dealloc(ptr, layout);
        }
        assert_eq!(allocator.live_bytes(), 0);
    }

    struct RejectResize;

    unsafe impl GlobalAlloc for RejectResize {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) };
        }

        unsafe fn realloc(&self, _ptr: *mut u8, _layout: Layout, _new_size: usize) -> *mut u8 {
            ptr::null_mut()
        }
    }

    #[test]
    fn failed_resize_preserves_live_bytes() {
        let allocator = CountingAllocator::new(RejectResize);
        let layout = Layout::from_size_align(64, 8).unwrap();

        unsafe {
            let ptr = allocator.alloc(layout);
            assert!(!ptr.is_null());
            assert!(allocator.realloc(ptr, layout, 256).is_null());
            assert_eq!(allocator.live_bytes(), layout.size());
            allocator.dealloc(ptr, layout);
        }
        assert_eq!(allocator.live_bytes(), 0);
    }

    #[test]
    fn counts_cross_thread_free() {
        let allocator = CountingAllocator::new(System);
        let layout = Layout::from_size_align(64, 8).unwrap();

        let ptr = unsafe { allocator.alloc(layout) };
        assert!(!ptr.is_null());
        assert_eq!(allocator.live_bytes(), layout.size());

        let ptr = AtomicPtr::new(ptr);
        thread::scope(|scope| {
            scope.spawn(|| {
                unsafe { allocator.dealloc(ptr.load(Ordering::Relaxed), layout) };
            });
        });
        assert_eq!(allocator.live_bytes(), 0);
    }
}
