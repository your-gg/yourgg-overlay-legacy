use asdf_overlay_event::{AugmentCard, AugmentChoices, OverlayEvent, OwnedAugments};
use std::{
    ffi::{CStr, c_char, c_void},
    ptr::NonNull,
    slice,
};

use crate::server::IpcClientEventEmitter;

#[repr(C)]
struct NativeAugmentCard {
    instance: u32,
    name: *const c_char,
    description: *const c_char,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

type ChoicesCallback =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const NativeAugmentCard, usize);
type OwnedCallback = unsafe extern "C" fn(*mut c_void, *const *const c_char, usize);
type ErrorCallback = unsafe extern "C" fn(*mut c_void, *const c_char);

unsafe extern "C" {
    fn yourgg_augment_reader_create() -> *mut c_void;
    fn yourgg_augment_reader_start(
        handle: *mut c_void,
        context: *mut c_void,
        on_choices: Option<ChoicesCallback>,
        on_owned: Option<OwnedCallback>,
        on_error: Option<ErrorCallback>,
    ) -> bool;
    fn yourgg_augment_reader_stop(handle: *mut c_void);
    fn yourgg_augment_reader_destroy(handle: *mut c_void);
}

pub struct AugmentReader {
    handle: NonNull<c_void>,
    context: NonNull<IpcClientEventEmitter>,
}

impl AugmentReader {
    pub fn start(emitter: IpcClientEventEmitter) -> anyhow::Result<Self> {
        let handle = NonNull::new(unsafe { yourgg_augment_reader_create() })
            .ok_or_else(|| anyhow::anyhow!("cannot create augment memory reader"))?;
        let context = NonNull::from(Box::leak(Box::new(emitter)));

        let started = unsafe {
            yourgg_augment_reader_start(
                handle.as_ptr(),
                context.as_ptr().cast(),
                Some(on_choices),
                Some(on_owned),
                Some(on_error),
            )
        };
        if !started {
            unsafe {
                yourgg_augment_reader_destroy(handle.as_ptr());
                drop(Box::from_raw(context.as_ptr()));
            }
            anyhow::bail!("cannot start augment memory reader");
        }

        Ok(Self { handle, context })
    }
}

impl Drop for AugmentReader {
    fn drop(&mut self) {
        unsafe {
            yourgg_augment_reader_stop(self.handle.as_ptr());
            yourgg_augment_reader_destroy(self.handle.as_ptr());
            drop(Box::from_raw(self.context.as_ptr()));
        }
    }
}

unsafe extern "C" fn on_choices(
    context: *mut c_void,
    mode: *const c_char,
    cards: *const NativeAugmentCard,
    card_count: usize,
) {
    let Some(emitter) = (unsafe { context.cast::<IpcClientEventEmitter>().as_ref() }) else {
        return;
    };
    if mode.is_null() || (card_count != 0 && cards.is_null()) {
        return;
    }

    let mode = unsafe { CStr::from_ptr(mode) }
        .to_string_lossy()
        .into_owned();
    let cards = if card_count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(cards, card_count) }
    };
    let cards = cards
        .iter()
        .map(|card| AugmentCard {
            instance: card.instance,
            name: copy_string(card.name),
            description: copy_string(card.description),
            x: card.x,
            y: card.y,
            width: card.width,
            height: card.height,
        })
        .collect();

    let _ = emitter.emit(OverlayEvent::LolAugmentChoices(AugmentChoices {
        mode,
        cards,
    }));
}

unsafe extern "C" fn on_owned(
    context: *mut c_void,
    names: *const *const c_char,
    name_count: usize,
) {
    let Some(emitter) = (unsafe { context.cast::<IpcClientEventEmitter>().as_ref() }) else {
        return;
    };
    if name_count != 0 && names.is_null() {
        return;
    }

    let names = if name_count == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(names, name_count) }
    };
    let internal_names = names.iter().copied().map(copy_string).collect();
    let _ = emitter.emit(OverlayEvent::LolAugmentOwned(OwnedAugments {
        internal_names,
    }));
}

unsafe extern "C" fn on_error(context: *mut c_void, error: *const c_char) {
    let Some(emitter) = (unsafe { context.cast::<IpcClientEventEmitter>().as_ref() }) else {
        return;
    };
    let _ = emitter.emit(OverlayEvent::LolAugmentReadError(copy_string(error)));
}

fn copy_string(value: *const c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}
