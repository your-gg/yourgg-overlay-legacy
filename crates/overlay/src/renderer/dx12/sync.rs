use windows::Win32::{
    Foundation::CloseHandle,
    Graphics::Direct3D12::*,
    System::Threading::{CreateEventW, WaitForSingleObject},
};

/// Maximum time (in milliseconds) to wait for a GPU fence to complete.
///
/// A GPU hang / TDR must never block the calling thread forever, so the wait is
/// bounded. On timeout we give up gracefully instead of deadlocking.
const FENCE_WAIT_TIMEOUT_MS: u32 = 5000;

#[derive(Debug)]
pub struct RendererFence {
    fence: ID3D12Fence,
    fence_val: u64,
}

impl RendererFence {
    pub fn new(device: &ID3D12Device) -> anyhow::Result<Self> {
        Ok(Self {
            fence: unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE)? },
            fence_val: 0,
        })
    }

    pub fn register(&mut self, queue: &ID3D12CommandQueue) -> anyhow::Result<()> {
        self.fence_val += 1;
        unsafe {
            queue.Signal(&self.fence, self.fence_val)?;
        }

        Ok(())
    }

    /// Latest fence value signalled by [`Self::register`].
    pub fn current_value(&self) -> u64 {
        self.fence_val
    }

    pub fn wait_pending(&self) -> anyhow::Result<()> {
        self.wait_value(self.fence_val)
    }

    /// Wait until the fence reaches `value`, bounded by a finite timeout.
    ///
    /// Returns `Ok(())` once the value is reached, on timeout, or if the device
    /// was removed (TDR) -- a GPU hang must never freeze the calling thread
    /// forever. The success path (fence already/eventually completed) is
    /// preserved exactly.
    pub fn wait_value(&self, value: u64) -> anyhow::Result<()> {
        unsafe {
            // Fast path: nothing to wait for.
            if self.fence.GetCompletedValue() >= value {
                return Ok(());
            }

            // Create a real (auto-reset, initially non-signalled) event to wait
            // on instead of blocking forever on a null handle.
            let event = match CreateEventW(None, false, false, None) {
                Ok(event) => event,
                // Could not create the event; skip waiting rather than block.
                Err(_) => return Ok(()),
            };

            // SetEventOnCompletion fails (e.g. DXGI_ERROR_DEVICE_REMOVED) when
            // the device was removed by a TDR. In that case the GPU work will
            // never complete, so skip waiting gracefully.
            if self.fence.SetEventOnCompletion(value, event).is_err() {
                let _ = CloseHandle(event);
                return Ok(());
            }

            // Bounded wait so a GPU hang cannot freeze this thread forever.
            // We give up regardless of the result (WAIT_OBJECT_0 = completed,
            // WAIT_TIMEOUT / WAIT_FAILED = GPU hang or error): the caller's
            // Reset/destroy proceeds rather than deadlocking.
            let _ = WaitForSingleObject(event, FENCE_WAIT_TIMEOUT_MS);
            let _ = CloseHandle(event);
        }
        Ok(())
    }
}

unsafe impl Send for RendererFence {}
