fn letscam_write_screenshot(
    this: &crate::gfx::Renderer,
    request: &crate::gfx::ScreenshotRequest,
    buffer: &wgpu::Buffer,
) -> core::ops::ControlFlow<Result<[u32; 2]>> {
    let Some(tag) = crate::letscam::capture_tag(&request.path) else {
        return core::ops::ControlFlow::Continue(());
    };
    let size = [this.config.width, this.config.height];
    let bgra = matches!(
        this.config.format.remove_srgb_suffix(),
        wgpu::TextureFormat::Bgra8Unorm
    );
    core::ops::ControlFlow::Break(match this.read_frame(buffer) {
        Ok(pixels) => {
            crate::letscam::put(tag, size, bgra, pixels);
            Ok(size)
        }
        Err(error) => Err(error),
    })
}
