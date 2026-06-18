use core::ffi::c_void;
use windows::{
    Win32::Graphics::{Direct3D::ID3DDestructionNotifier, Dxgi::IDXGISwapChain1},
    core::Interface,
};

pub fn register_swapchain_destruction_callback<F: FnOnce(usize) + Send + 'static>(
    swapchain: &IDXGISwapChain1,
    f: F,
) {
    struct Data<F> {
        this: usize,
        f: F,
    }

    #[tracing::instrument]
    extern "system" fn callback<F: FnOnce(usize)>(this: *mut c_void) {
        let this = unsafe { Box::from_raw(this.cast::<Data<F>>()) };
        (this.f)(this.this)
    }

    let Ok(notifier) = swapchain.cast::<ID3DDestructionNotifier>() else {
        return;
    };

    // Hand the boxed data to the destruction callback as a raw pointer. On a
    // successful registration the callback reclaims it via `Box::from_raw` when
    // the swapchain is destroyed. If registration fails the callback will never
    // fire, so reclaim the box here instead of leaking it (and the closure).
    let data = Box::into_raw(Box::new(Data {
        this: swapchain.as_raw() as _,
        f,
    }));
    // register with swapchain pointer without increasing ref
    let registered =
        unsafe { notifier.RegisterDestructionCallback(Some(callback::<F>), data as *mut _ as _) };
    if registered.is_err() {
        drop(unsafe { Box::from_raw(data) });
    }
}
