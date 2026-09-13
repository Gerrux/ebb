//! Renderer selection, fixed at build time: `--features glow` (default) or
//! `--no-default-features --features wgpu` (Direct3D 12 only).

#[cfg(feature = "wgpu")]
pub const NAME: &str = "wgpu-dx12";
#[cfg(not(feature = "wgpu"))]
pub const NAME: &str = "glow";

pub fn configure(options: &mut eframe::NativeOptions) {
    #[cfg(feature = "wgpu")]
    {
        use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetupCreateNew};
        use eframe::wgpu;

        options.renderer = eframe::Renderer::Wgpu;
        let mut setup = WgpuSetupCreateNew::without_display_handle();
        setup.instance_descriptor.backends = wgpu::Backends::DX12;
        // A plain HWND swapchain only supports opaque presentation; transparent
        // windows need a DirectComposition visual. Env var still wins for experiments.
        if std::env::var_os("WGPU_DX12_PRESENTATION_SYSTEM").is_none() {
            setup.instance_descriptor.backend_options.dx12.presentation_system =
                wgpu::Dx12SwapchainKind::DxgiFromVisual;
        }
        // The default `Performance` hints pre-allocate large heaps: 164/320 MiB
        // (working set/private) versus 105/141 MiB with `MemoryUsage`.
        let base = setup.device_descriptor.clone();
        setup.device_descriptor = std::sync::Arc::new(move |adapter| wgpu::DeviceDescriptor {
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..base(adapter)
        });
        options.wgpu_options = WgpuConfiguration {
            wgpu_setup: setup.into(),
            ..Default::default()
        };
    }
    #[cfg(not(feature = "wgpu"))]
    {
        options.renderer = eframe::Renderer::Glow;
    }
}
