use crate::KeyMap;
use fontdb::{Family, Query};
pub use kime_engine_config::*;
use std::collections::BTreeMap;
use std::fs;

fn build_category_hotkeys(
    engine: &EngineConfig,
    global_hotkeys: &BTreeMap<Key, Hotkey>,
) -> EnumMap<InputCategory, Vec<(Key, Hotkey)>> {
    enum_map! {
        category => {
            let mut hotkeys = engine
                .category_hotkeys
                .get(&category)
                .cloned()
                .unwrap_or_default();
            for (key, hotkey) in global_hotkeys {
                hotkeys.entry(*key).or_insert(*hotkey);
            }
            hotkeys.into_iter().collect()
        }
    }
}

fn build_mode_hotkeys(
    engine: &EngineConfig,
    global_hotkeys: &BTreeMap<Key, Hotkey>,
) -> EnumMap<InputMode, Vec<(Key, Hotkey)>> {
    enum_map! {
        mode => {
            let mut hotkeys = engine
                .mode_hotkeys
                .get(&mode)
                .cloned()
                .unwrap_or_default();
            for (key, hotkey) in global_hotkeys {
                hotkeys.entry(*key).or_insert(*hotkey);
            }
            hotkeys.into_iter().collect()
        }
    }
}

/// Preprocessed engine config
pub struct Config {
    pub translation_layer: Option<KeyMap<Key>>,
    pub default_category: InputCategory,
    pub global_category_state: bool,
    pub category_hotkeys: EnumMap<InputCategory, Vec<(Key, Hotkey)>>,
    pub mode_hotkeys: EnumMap<InputMode, Vec<(Key, Hotkey)>>,
    pub game_category_hotkeys: EnumMap<InputCategory, Vec<(Key, Hotkey)>>,
    pub game_mode_hotkeys: EnumMap<InputMode, Vec<(Key, Hotkey)>>,
    pub candidate_font: (Vec<u8>, u32),
    pub xim_preedit_font: (Vec<u8>, u32, f32),
    pub hangul_data: HangulData,
    pub preferred_direct: bool,
    pub latin_data: LatinData,
}

impl Default for Config {
    fn default() -> Self {
        Self::new(EngineConfig::default())
    }
}

impl Config {
    fn new_impl(engine: EngineConfig, hangul_data: HangulData) -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();

        let load_font = |name| {
            db.query(&Query {
                families: &[Family::Name(name), Family::Name("D2Coding")],
                ..Default::default()
            })
            .and_then(|id| db.with_face_data(id, |data, index| (data.to_vec(), index)))
            .unwrap_or_default()
        };

        #[cfg(unix)]
        let translation_layer: Option<KeyMap<Key>> = engine
            .translation_layer
            .as_ref()
            .and_then(|f| xdg::BaseDirectories::with_prefix("kime").find_config_file(f))
            .as_ref()
            .and_then(|f| fs::read_to_string(f.as_path()).ok())
            .as_ref()
            .and_then(|content| serde_yaml::from_str(content).ok());

        #[cfg(not(unix))]
        let translation_layer = None;

        let category_hotkeys = build_category_hotkeys(&engine, &engine.global_hotkeys);
        let mode_hotkeys = build_mode_hotkeys(&engine, &engine.global_hotkeys);
        let game_category_hotkeys = build_category_hotkeys(&engine, &engine.game_global_hotkeys);
        let game_mode_hotkeys = build_mode_hotkeys(&engine, &engine.game_global_hotkeys);

        Self {
            translation_layer: translation_layer,
            default_category: engine.default_category,
            global_category_state: engine.global_category_state,
            category_hotkeys,
            mode_hotkeys,
            game_category_hotkeys,
            game_mode_hotkeys,
            xim_preedit_font: {
                let (font, index) = load_font(&engine.xim_preedit_font.0);
                (font, index, engine.xim_preedit_font.1)
            },
            candidate_font: {
                let (font, index) = load_font(&engine.candidate_font);
                (font, index)
            },
            preferred_direct: engine.latin.preferred_direct,
            latin_data: LatinData::new(&engine.latin),
            hangul_data,
        }
    }

    pub fn new(engine: EngineConfig) -> Self {
        let hangul_data = HangulData::new(
            &engine.hangul,
            kime_engine_backend_hangul::builtin_layouts(),
        );

        Self::new_impl(engine, hangul_data)
    }

    #[cfg(unix)]
    pub fn from_engine_config_with_dir(engine: EngineConfig, dir: &xdg::BaseDirectories) -> Self {
        let hangul_data = HangulData::from_config_with_dir(&engine.hangul, dir);
        Self::new_impl(engine, hangul_data)
    }
}

#[cfg(unix)]
pub fn load_raw_config_from_config_dir() -> RawConfig {
    let dir = xdg::BaseDirectories::with_prefix("kime");

    dir.find_config_file("config.yaml")
        .and_then(|config| serde_yaml::from_reader(std::fs::File::open(config).ok()?).ok())
        .unwrap_or_default()
}

#[cfg(unix)]
pub fn load_engine_config_from_config_dir() -> Option<Config> {
    let dir = xdg::BaseDirectories::with_prefix("kime");
    let config: RawConfig = dir
        .find_config_file("config.yaml")
        .and_then(|config| serde_yaml::from_reader(std::fs::File::open(config).ok()?).ok())
        .unwrap_or_default();

    Some(Config::from_engine_config_with_dir(config.engine, &dir))
}
