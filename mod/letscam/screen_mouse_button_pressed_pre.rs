fn letscam_screen_mouse_button_pressed_pre(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    crate::game::letscam_screen::on_press(game, screen, button, canceled);
}
