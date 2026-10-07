//! Per-thread allocation accounting for large-file regression tests.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! { static BYTES: Cell<Option<usize>> = const { Cell::new(None) }; }
struct CountingAllocator;
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
fn record(size: usize) {
    let _ = BYTES.try_with(|count| {
        if let Some(total) = count.get() {
            count.set(Some(total.saturating_add(size)));
        }
    });
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);
        System.realloc(ptr, layout, size)
    }
}
pub(crate) fn measure<T>(action: impl FnOnce() -> T) -> (T, usize) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            BYTES.with(|count| count.set(None));
        }
    }
    BYTES.with(|count| count.set(Some(0)));
    let _reset = Reset;
    let result = action();
    let count = BYTES.with(|count| count.get().unwrap());
    (result, count)
}
