//! The commented settings file written on first start.

use crate::config::{Codec, Config, Container};

fn quoted(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn codec_name(c: Codec) -> &'static str {
    match c {
        Codec::H264 => "h264",
        Codec::Hevc => "hevc",
        Codec::Av1 => "av1",
    }
}

fn container_name(c: Container) -> &'static str {
    c.extension()
}

/// Settings file text with a Russian comment above every key.
pub(crate) fn render(cfg: &Config) -> String {
    let d = &cfg.director;
    format!(
        r#"# LetsCam — запись летсплеев. Файл настроек.
# Если удалить строку, будет взято значение по умолчанию.
# Если файл сломан, мод возьмёт значения по умолчанию и напишет об этом в чат.

# Путь к ffmpeg. "ffmpeg" — взять из PATH. Например: "C:/ffmpeg/bin/ffmpeg.exe".
ffmpeg = {ffmpeg}
# Папка для записей. Пусто — папка letscam внутри папки игры.
output_dir = {output_dir}
# Размер ролика в пикселях: 1280x720, 1920x1080, 2560x1440 или 3840x2160.
# Если окно игры другого размера, кадр масштабируется.
width = {width}
height = {height}
# Кадров в секунду в ролике: 30 или 60.
fps = {fps}
# Видеокодек: "h264", "hevc" или "av1".
codec = "{codec}"
# true — кодировать на видеокарте NVIDIA (NVENC). Если NVENC нет, мод сам перейдёт на процессор.
hardware = {hardware}
# Качество: меньше — лучше и больше файл. 18–28 — разумно. Для NVENC это CQ, для процессора CRF.
quality = {quality}
# Контейнер: "mp4" (звук AAC) или "mkv" (звук FLAC без потерь).
container = "{container}"
# Записывать микрофон отдельной дорожкой.
record_mic = {record_mic}
# Микрофон: часть его названия, например "Blue Yeti". Пусто — микрофон по умолчанию.
mic_device = {mic_device}
# Оставлять черновики (game.wav, mic.wav, gui/) после рендера. Реплей остаётся всегда.
keep_sources = {keep_sources}
# Сколько раз в секунду снимать открытый интерфейс (инвентарь, сундук).
gui_fps = {gui_fps}

# Камера-режиссёр. Расстояния в блоках, время в секундах, углы в градусах.
[director]
# Сглаживание движения камеры (пружина), сек.
spring_s = {spring_s:?}
# Сглаживание поворота камеры, сек.
look_spring_s = {look_spring_s:?}
# Угол обзора камеры, градусы.
fov = {fov:?}
# Следование: расстояние от глаз до камеры и из него высота над глазами.
follow_distance = {follow_distance:?}
follow_height = {follow_height:?}
# Камера смотрит на точку на столько блоков впереди игрока.
look_ahead = {look_ahead:?}
# След: насколько назад по времени искать точку на пройденном пути (чтобы камера входила в двери).
trail_s = {trail_s:?}
# Смена плеча: пауза между сменами, угол вбок, сколько держаться у плеча, время перехода.
shoulder_every_min_s = {every_min:?}
shoulder_every_max_s = {every_max:?}
shoulder_angle_min_deg = {angle_min:?}
shoulder_angle_max_deg = {angle_max:?}
shoulder_hold_min_s = {hold_min:?}
shoulder_hold_max_s = {hold_max:?}
shoulder_move_s = {shoulder_move:?}
# Потолок: высота проверки над глазами, запас под потолком, ближе всего к игроку.
ceiling_probe = {ceiling_probe:?}
ceiling_margin = {ceiling_margin:?}
ceiling_min_distance = {ceiling_min:?}
# Стены: запас до блока, шаги поиска, время отхода назад (подход мгновенный).
wall_margin = {wall_margin:?}
wall_iterations = {wall_iterations}
wall_push_out_s = {wall_push:?}
# Интерфейс открыт: облёт вокруг игрока — радиус, высота, секунд на полный круг.
gui_radius = {gui_radius:?}
gui_height = {gui_height:?}
gui_period_s = {gui_period:?}
# Общий план (начало, телепорт, смена измерения): длительность, начальная высота, дальность телепорта.
establishing_s = {est_s:?}
establishing_height = {est_h:?}
teleport_distance = {teleport:?}
# Бой: сколько секунд после удара, расстояние, высота относительно глаз.
combat_window_s = {combat_window:?}
combat_distance = {combat_distance:?}
combat_height = {combat_height:?}
# Полёт на элитрах: расстояние, высота, угол вбок.
elytra_distance = {elytra_distance:?}
elytra_height = {elytra_height:?}
elytra_side_deg = {elytra_side:?}
# Стоит на месте: через сколько секунд, радиус облёта, высота, секунд на круг, порог движения.
idle_after_s = {idle_after:?}
idle_radius = {idle_radius:?}
idle_height = {idle_height:?}
idle_period_s = {idle_period:?}
idle_move_epsilon = {idle_eps:?}
# Смешивание при смене режима камеры, сек.
blend_s = {blend:?}
"#,
        ffmpeg = quoted(&cfg.ffmpeg),
        output_dir = quoted(&cfg.output_dir),
        width = cfg.width,
        height = cfg.height,
        fps = cfg.fps,
        codec = codec_name(cfg.codec),
        hardware = cfg.hardware,
        quality = cfg.quality,
        container = container_name(cfg.container),
        record_mic = cfg.record_mic,
        mic_device = quoted(&cfg.mic_device),
        keep_sources = cfg.keep_sources,
        gui_fps = cfg.gui_fps,
        spring_s = d.spring_s,
        look_spring_s = d.look_spring_s,
        fov = d.fov,
        follow_distance = d.follow_distance,
        follow_height = d.follow_height,
        look_ahead = d.look_ahead,
        trail_s = d.trail_s,
        every_min = d.shoulder_every_min_s,
        every_max = d.shoulder_every_max_s,
        angle_min = d.shoulder_angle_min_deg,
        angle_max = d.shoulder_angle_max_deg,
        hold_min = d.shoulder_hold_min_s,
        hold_max = d.shoulder_hold_max_s,
        shoulder_move = d.shoulder_move_s,
        ceiling_probe = d.ceiling_probe,
        ceiling_margin = d.ceiling_margin,
        ceiling_min = d.ceiling_min_distance,
        wall_margin = d.wall_margin,
        wall_iterations = d.wall_iterations,
        wall_push = d.wall_push_out_s,
        gui_radius = d.gui_radius,
        gui_height = d.gui_height,
        gui_period = d.gui_period_s,
        est_s = d.establishing_s,
        est_h = d.establishing_height,
        teleport = d.teleport_distance,
        combat_window = d.combat_window_s,
        combat_distance = d.combat_distance,
        combat_height = d.combat_height,
        elytra_distance = d.elytra_distance,
        elytra_height = d.elytra_height,
        elytra_side = d.elytra_side_deg,
        idle_after = d.idle_after_s,
        idle_radius = d.idle_radius,
        idle_height = d.idle_height,
        idle_period = d.idle_period_s,
        idle_eps = d.idle_move_epsilon,
        blend = d.blend_s,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_back_to_defaults() {
        let cfg = Config::default();
        let parsed: Config = toml::from_str(&render(&cfg)).expect("template parses");
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn template_mentions_every_key() {
        let cfg = Config::default();
        let template: toml::Table = toml::from_str(&render(&cfg)).expect("parse");
        let full: toml::Table =
            toml::from_str(&toml::to_string(&cfg).expect("ser")).expect("parse");
        for (k, v) in &full {
            assert!(template.contains_key(k), "template misses key {k}");
            if let (Some(inner), Some(tpl_inner)) =
                (v.as_table(), template.get(k).and_then(|t| t.as_table()))
            {
                for key in inner.keys() {
                    assert!(tpl_inner.contains_key(key), "template misses {k}.{key}");
                }
            }
        }
    }

    #[test]
    fn template_escapes_strings() {
        let cfg = Config {
            ffmpeg: "C:\\ffmpeg\\bin\\ffmpeg.exe".into(),
            mic_device: "Mic \"USB\"".into(),
            ..Config::default()
        };
        let parsed: Config = toml::from_str(&render(&cfg)).expect("parse");
        assert_eq!(parsed.ffmpeg, cfg.ffmpeg);
        assert_eq!(parsed.mic_device, cfg.mic_device);
    }
}
