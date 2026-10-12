use reveler_dsp::{Parameters, Processor};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct GuardedAllocator;
thread_local! {static TRACK: Cell<bool>=const {Cell::new(false)};static COUNT:Cell<usize>=const {Cell::new(0)};}
fn allocation() {
    TRACK.with(|t| {
        if t.get() {
            COUNT.with(|c| c.set(c.get() + 1));
        }
    });
}
unsafe impl GlobalAlloc for GuardedAllocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        allocation();
        unsafe { System.realloc(p, l, n) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc_zeroed(l) }
    }
}
#[global_allocator]
static ALLOCATOR: GuardedAllocator = GuardedAllocator;

#[test]
fn process_parameter_changes_and_reset_do_not_allocate() {
    let mut dsp = Processor::new(192000.0, 6).unwrap();
    COUNT.with(|c| c.set(0));
    TRACK.with(|t| t.set(true));
    for i in 0..4000 {
        if i % 200 == 0 {
            dsp.set_parameters(Parameters {
                true_peak_db: -1.0,
                max_gain_db: (i % 30) as f64,
                ..Parameters::default()
            });
        }
        let mut frame = [(i as f32 * 0.13).sin(); 6];
        dsp.process_frame(&mut frame);
    }
    dsp.reset();
    TRACK.with(|t| t.set(false));
    assert_eq!(COUNT.with(|c| c.get()), 0);
}
