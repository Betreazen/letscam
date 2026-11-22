// A `letscam-capture:` request's readback goes to LetsCam's hand-off instead of a file.
fn letscam_save_screenshot(
    pending: &mut horizon_client::gfx::PendingScreenshot,
) -> core::ops::ControlFlow<Option<std::result::Result<[u32; 2], String>>> {
    let Some(tag) = letscam_engine::bridge::capture_tag(&pending.path) else {
        return core::ops::ControlFlow::Continue(());
    };
    let image = pending.image_mut();
    let size = [image.width, image.height];
    letscam_engine::bridge::put(tag, size, false, ::std::mem::take(&mut image.rgba));
    core::ops::ControlFlow::Break(Some(Ok(size)))
}
