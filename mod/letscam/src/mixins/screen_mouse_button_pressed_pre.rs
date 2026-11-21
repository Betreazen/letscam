fn letscam_screen_mouse_button_pressed_pre(
    game: &Game<'_>,
    screen: &ScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    crate::game::letscam_screen::on_press(game, screen, button, canceled);
}
