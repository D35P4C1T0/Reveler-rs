use std::cmp;
#[cfg(feature = "editor")]
use vst3::Steinberg::FIDString;
use vst3::Steinberg::Vst::TChar;
use widestring::U16CString;

/// When `Plugin::MIDI_INPUT` is set to `MidiConfig::MidiCCs` or higher then we'll register 130*16
/// additional parameters to handle MIDI CCs, channel pressure, and pitch bend, in that order.
/// vst3 doesn't expose these constants.
pub const VST3_MIDI_CCS: u32 = 130;
pub const VST3_MIDI_CHANNELS: u32 = 16;
/// The number of parameters we'll need to register if the plugin accepts MIDI CCs.
pub const VST3_MIDI_NUM_PARAMS: u32 = VST3_MIDI_CCS * VST3_MIDI_CHANNELS;
/// The start of the MIDI CC parameter ranges. We'll print an assertion failure if any of the
/// plugin's parameters overlap with this range. The mapping to a parameter index is
/// `VST3_MIDI_PARAMS_START + (cc_idx + (channel * VST3_MIDI_CCS))`.
pub const VST3_MIDI_PARAMS_START: u32 = VST3_MIDI_PARAMS_END - VST3_MIDI_NUM_PARAMS;
/// The (exclusive) end of the MIDI CC parameter range. Anything above this is reserved by the host.
pub const VST3_MIDI_PARAMS_END: u32 = 1 << 31;

/// Early exit out of a VST3 function when one of the passed pointers is null
macro_rules! check_null_ptr {
    ($ptr:expr $(, $ptrs:expr)* $(, )?) => {
        check_null_ptr_msg!("Null pointer passed to function", $ptr $(, $ptrs)*)
    };
}

/// The same as [`check_null_ptr!`], but with a custom message.
macro_rules! check_null_ptr_msg {
    ($msg:expr, $ptr:expr $(, $ptrs:expr)* $(, )?) => {
        if $ptr.is_null() $(|| $ptrs.is_null())* {
            crate::nice_debug_assert_failure!($msg);
            return kInvalidArgument;
        }
    };
}

/// The same as [`strlcpy()`], but for VST3's fun UTF-16 strings instead.
pub fn u16strlcpy(dest: &mut [TChar], src: &str) {
    if dest.is_empty() {
        return;
    }

    let src_utf16 = match U16CString::from_str(src) {
        Ok(s) => s,
        Err(err) => {
            crate::nice_debug_assert_failure!("Invalid UTF-16 string: {}", err);
            return;
        }
    };
    let src_utf16_chars = src_utf16.as_slice();

    // Make sure there's always room for a null terminator
    let copy_len = cmp::min(dest.len() - 1, src_utf16_chars.len());
    dest[..copy_len].copy_from_slice(&src_utf16_chars[..copy_len]);
    dest[copy_len] = 0;
}

/// Compare a host-provided [`FIDString`] to one of the SDK's string constants. These are plain C
/// string pointers, so pointer equality is not sufficient.
#[cfg(feature = "editor")]
pub unsafe fn fid_matches(type_: FIDString, expected: FIDString) -> bool {
    use std::ffi::CStr;

    unsafe { !type_.is_null() && CStr::from_ptr(type_) == CStr::from_ptr(expected) }
}

/// Host run-loop callbacks are not necessarily delivered on the editor's owning
/// thread (notably in Carla on Linux). Never unwind through the VST3 ABI or move
/// a thread-bound editor to a worker thread when a host violates that contract.
#[cfg(feature = "editor")]
pub(super) fn with_editor_window<T>(
    window: &atomic_refcell::AtomicRefCell<Option<fragile::Fragile<T>>>,
    callback: impl FnOnce(&T),
) {
    let Ok(window) = window.try_borrow() else {
        crate::nice_warn!("Ignoring VST3 editor callback while the editor is being changed");
        return;
    };
    let Some(window) = window.as_ref() else {
        return;
    };
    let Ok(window) = window.try_get() else {
        crate::nice_warn!(
            "Ignoring VST3 editor callback on a thread other than the editor's owning thread"
        );
        return;
    };
    callback(window);
}

#[cfg(all(test, feature = "editor"))]
mod editor_thread_tests {
    use super::with_editor_window;
    use atomic_refcell::AtomicRefCell;
    use fragile::Fragile;
    use std::{cell::Cell, rc::Rc, sync::Arc};

    #[test]
    fn foreign_thread_callback_does_not_panic_or_touch_editor() {
        // Rc<Cell<_>> deliberately models an editor that is neither Send nor Sync.
        let editor = Rc::new(Cell::new(0));
        let window = Arc::new(AtomicRefCell::new(Some(Fragile::new(editor.clone()))));
        let foreign = window.clone();
        std::thread::spawn(move || {
            with_editor_window(&foreign, |_| panic!("accessed editor on foreign thread"));
        })
        .join()
        .unwrap();
        assert_eq!(editor.get(), 0);
        with_editor_window(&window, |editor| editor.set(1));
        assert_eq!(editor.get(), 1);
    }

    #[test]
    fn closed_and_mutably_borrowed_editors_skip_callbacks() {
        let window = AtomicRefCell::new(None::<Fragile<()>>);
        with_editor_window(&window, |_| panic!("accessed closed editor"));
        *window.borrow_mut() = Some(Fragile::new(()));
        let changing_editor = window.borrow_mut();
        with_editor_window(&window, |_| panic!("accessed editor during mutation"));
        drop(changing_editor);
        let mut called = false;
        with_editor_window(&window, |_| called = true);
        assert!(called);
    }
}

#[cfg(test)]
mod miri {
    use widestring::U16CStr;

    use super::*;

    #[test]
    fn u16strlcpy_normal() {
        let mut dest = [0; 256];
        u16strlcpy(&mut dest, "Hello, world!");

        assert_eq!(
            unsafe { U16CStr::from_ptr_str(dest.as_ptr() as *const u16) }
                .to_string()
                .unwrap(),
            "Hello, world!"
        );
    }

    #[test]
    fn u16strlcpy_overflow() {
        let mut dest = [0; 6];
        u16strlcpy(&mut dest, "Hello, world!");

        assert_eq!(
            unsafe { U16CStr::from_ptr_str(dest.as_ptr() as *const u16) }
                .to_string()
                .unwrap(),
            "Hello"
        );
    }
}
