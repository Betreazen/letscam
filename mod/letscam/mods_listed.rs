fn letscam_mods_listed(mods: &mut Vec<crate::mod_api::NativeMod>) {
    crate::game::letscam_glue::on_mods(mods);
}
