use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::pe_window::PeWindow;
use ab_glyph::{FontArc, FontVec};
use ahash::AHashMap;
use kime_engine_core::{
    Config, InputEngine, InputResult, Key, KeyCode, ModifierState, RuntimeProfile,
};
use x11rb::{
    connection::Connection,
    protocol::xproto::{ConfigureNotifyEvent, KeyButMask, KeyPressEvent, KEY_PRESS_EVENT},
    protocol::xtest::ConnectionExt as XTestConnectionExt,
    CURRENT_TIME,
};
use xim::{
    x11rb::{HasConnection, X11rbServer},
    InputStyle, Server, ServerHandler,
};

pub struct KimeData {
    engine: InputEngine,
    profile: Arc<AtomicU8>,
    pe: Option<NonZeroU32>,
    show_preedit_window: bool,
    engine_ready: bool,
}

impl KimeData {
    pub fn new(config: &Config, show_preedit_window: bool, profile: Arc<AtomicU8>) -> Self {
        Self {
            engine: InputEngine::new(config),
            profile,
            pe: None,
            show_preedit_window,
            engine_ready: true,
        }
    }
}

pub struct KimeHandler {
    preedit_windows: AHashMap<NonZeroU32, PeWindow>,
    xim_repeats: AHashMap<u64, XimRepeat>,
    font: Option<(FontArc, f32)>,
    config: Config,
    profile: Arc<AtomicU8>,
    screen_num: usize,
}

#[derive(Clone, Copy)]
struct XimRepeat {
    keycode: u8,
    event: KeyPressEvent,
    next: Instant,
    pending: u32,
}

const XIM_REPEAT_DELAY: Duration = Duration::from_millis(400);
const XIM_REPEAT_INTERVAL: Duration = Duration::from_millis(50);
const SEND_EVENT_FLAG: u8 = 0x80;

impl KimeHandler {
    #[cfg(test)]
    pub fn new(screen_num: usize, config: Config) -> Self {
        Self::with_profile(
            screen_num,
            config,
            Arc::new(AtomicU8::new(RuntimeProfile::Normal.as_u8())),
        )
    }

    pub fn with_profile(screen_num: usize, config: Config, profile: Arc<AtomicU8>) -> Self {
        let (font_data, index, font_size) = &config.xim_preedit_font;
        // The preedit font can be missing (e.g. neither the configured
        // `xim_preedit_font` nor the fallback is installed), in which case
        // `load_font` yields empty data and parsing fails. Degrade gracefully
        // by disabling the preedit popup window instead of panicking. See #706.
        let font = match FontVec::try_from_vec_and_index(font_data.clone(), *index) {
            Ok(font_vec) => Some((FontArc::from(font_vec), *font_size)),
            Err(err) => {
                log::warn!(
                    "Failed to load xim preedit font; preedit window is disabled: {}",
                    err
                );
                None
            }
        };

        Self {
            preedit_windows: AHashMap::new(),
            xim_repeats: AHashMap::new(),
            screen_num,
            font,
            config,
            profile,
        }
    }
}

impl KimeHandler {
    fn repeat_id(user_ic: &xim::UserInputContext<KimeData>) -> u64 {
        (u64::from(user_ic.ic.client_win()) << 16) | u64::from(user_ic.ic.input_context_id().get())
    }

    fn is_configured_hotkey(&self, key: Key) -> bool {
        let profile = RuntimeProfile::from_u8(self.profile.load(Ordering::Relaxed));
        let (category_hotkeys, mode_hotkeys) = match profile {
            RuntimeProfile::Normal => (&self.config.category_hotkeys, &self.config.mode_hotkeys),
            RuntimeProfile::Game => (
                &self.config.game_category_hotkeys,
                &self.config.game_mode_hotkeys,
            ),
        };

        category_hotkeys
            .iter()
            .any(|(_, hotkeys)| hotkeys.iter().any(|(candidate, _)| *candidate == key))
            || mode_hotkeys
                .iter()
                .any(|(_, hotkeys)| hotkeys.iter().any(|(candidate, _)| *candidate == key))
    }

    fn can_repeat(&self, key: Key) -> bool {
        if matches!(
            key.code,
            KeyCode::Shift
                | KeyCode::ControlL
                | KeyCode::ControlR
                | KeyCode::AltL
                | KeyCode::AltR
                | KeyCode::SuperL
                | KeyCode::SuperR
        ) {
            return false;
        }

        !self.is_configured_hotkey(key)
    }

    fn stop_repeat(&mut self, user_ic: &xim::UserInputContext<KimeData>) {
        self.xim_repeats.remove(&Self::repeat_id(user_ic));
    }

    fn start_repeat(&mut self, user_ic: &xim::UserInputContext<KimeData>, xev: KeyPressEvent) {
        // The event window is the window on which the client originally
        // received the key. Fall back to the focus/client window for clients
        // that leave it unset.
        let target = if xev.event != 0 {
            xev.event
        } else {
            user_ic
                .ic
                .app_focus_win()
                .or_else(|| user_ic.ic.app_win())
                .map(NonZeroU32::get)
                .unwrap_or(0)
        };

        if target == 0 {
            log::debug!("Cannot start XIM repeat without a target window");
            return;
        }

        let mut event = xev;
        event.event = target;
        event.response_type = KEY_PRESS_EVENT;
        self.xim_repeats.insert(
            Self::repeat_id(user_ic),
            XimRepeat {
                keycode: xev.detail,
                event,
                next: Instant::now() + XIM_REPEAT_DELAY,
                pending: 0,
            },
        );
        log::trace!("Start XIM repeat for keycode {}", xev.detail);
    }

    pub fn repeat_timeout(&self) -> Option<Duration> {
        let now = Instant::now();
        self.xim_repeats
            .values()
            .map(|repeat| repeat.next.saturating_duration_since(now))
            .min()
    }

    pub fn clear_repeats(&mut self) {
        self.xim_repeats.clear();
    }

    pub fn emit_repeats<C>(&mut self, conn: &C) -> Result<(), xim::ServerError>
    where
        C: Connection + XTestConnectionExt,
    {
        let now = Instant::now();
        for repeat in self.xim_repeats.values_mut() {
            if repeat.next > now {
                continue;
            }

            conn.xtest_fake_input(
                KEY_PRESS_EVENT,
                repeat.keycode,
                CURRENT_TIME,
                repeat.event.root,
                repeat.event.root_x,
                repeat.event.root_y,
                0,
            )?;
            repeat.pending = repeat.pending.saturating_add(1);

            // Do not replay missed ticks after a scheduling hiccup. One
            // synthetic press per interval is enough and avoids a burst of
            // buffered characters when the desktop is briefly busy.
            repeat.next = now + XIM_REPEAT_INTERVAL;
        }

        Ok(())
    }

    pub fn expose(&mut self, window: u32, conn: &impl Connection) -> Result<(), xim::ServerError> {
        if let Some(win) = NonZeroU32::new(window) {
            if let Some(pe) = self.preedit_windows.get_mut(&win) {
                pe.expose(conn)?;
            }
        }

        Ok(())
    }

    pub fn configure_notify(
        &mut self,
        e: ConfigureNotifyEvent,
        conn: &impl Connection,
    ) -> Result<(), xim::ServerError> {
        if let Some(win) = NonZeroU32::new(e.window) {
            if let Some(pe) = self.preedit_windows.get_mut(&win) {
                pe.configure_notify(e, conn)?;
            }
        }

        Ok(())
    }

    fn preedit_draw<C: HasConnection>(
        &mut self,
        server: &mut X11rbServer<C>,
        ic: &mut xim::UserInputContext<KimeData>,
    ) -> Result<(), xim::ServerError> {
        server.preedit_draw(&mut ic.ic, ic.user_data.engine.preedit_str())?;
        Ok(())
    }

    fn preedit<C: HasConnection>(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<KimeData>,
    ) -> Result<(), xim::ServerError> {
        if user_ic
            .ic
            .input_style()
            .contains(InputStyle::PREEDIT_CALLBACKS)
        {
            self.preedit_draw(server, user_ic)?;
            return Ok(());
        }

        if !user_ic.user_data.show_preedit_window {
            return Ok(());
        }

        if user_ic.user_data.engine.preedit_str().is_empty() {
            return Ok(());
        }

        // No usable preedit font was loaded; skip the popup window. See #706.
        let font = match self.font.clone() {
            Some(font) => font,
            None => return Ok(()),
        };

        if let Some(pe) = user_ic.user_data.pe.as_mut() {
            // Draw in server (already have pe_window)
            let pe = self.preedit_windows.get_mut(pe).unwrap();
            pe.set_preedit(user_ic.user_data.engine.preedit_str());
            pe.refresh(server.conn())?;
        } else {
            // Draw in server
            let mut pe = PeWindow::new(
                server.conn(),
                font,
                user_ic.ic.app_win(),
                user_ic.ic.preedit_spot(),
                self.screen_num,
            )?;

            pe.set_preedit(user_ic.user_data.engine.preedit_str());
            user_ic.user_data.pe = Some(pe.window());

            self.preedit_windows.insert(pe.window(), pe);
        }

        Ok(())
    }

    fn reset<C: HasConnection>(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<KimeData>,
    ) -> Result<(), xim::ServerError> {
        user_ic.user_data.engine.clear_preedit();

        self.clear_preedit(server, user_ic)?;
        self.commit(server, user_ic)?;

        user_ic.user_data.engine.reset();

        Ok(())
    }

    fn process_input_result<C: HasConnection>(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<KimeData>,
        ret: InputResult,
    ) -> Result<bool, xim::ServerError> {
        log::trace!("{:?}", ret);

        if ret.contains(InputResult::LANGUAGE_CHANGED) {
            user_ic.user_data.engine.update_layout_state().ok();
        }

        if !ret.contains(InputResult::HAS_PREEDIT) {
            self.clear_preedit(server, user_ic)?;
        }

        if ret.contains(InputResult::HAS_COMMIT) {
            self.commit(server, user_ic)?;
            user_ic.user_data.engine.clear_commit();
        }

        if ret.contains(InputResult::HAS_PREEDIT) {
            self.preedit(server, user_ic)?;
        }

        user_ic.user_data.engine_ready = !ret.contains(InputResult::NOT_READY);

        Ok(ret.contains(InputResult::CONSUMED))
    }

    fn clear_preedit<C: HasConnection>(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<KimeData>,
    ) -> Result<(), xim::ServerError> {
        if user_ic
            .ic
            .input_style()
            .contains(InputStyle::PREEDIT_CALLBACKS)
        {
            server.preedit_draw(&mut user_ic.ic, "")?;
            return Ok(());
        }

        if let Some(pe) = user_ic.user_data.pe.take() {
            // off-the-spot draw in server
            if let Some(w) = self.preedit_windows.remove(&pe) {
                log::trace!("Destroy PeWindow: {}", w.window());
                w.clean(server.conn())?;
            }
        }

        Ok(())
    }

    fn commit<C: HasConnection>(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<KimeData>,
    ) -> Result<(), xim::ServerError> {
        self.clear_preedit(server, user_ic)?;
        let s = user_ic.user_data.engine.commit_str();
        if !s.is_empty() {
            server.commit(&user_ic.ic, s)?;
        }
        Ok(())
    }
}

// PRESS | RELEASE
const EVENT_MASK: u32 = 3;

impl<C: HasConnection> ServerHandler<X11rbServer<C>> for KimeHandler {
    type InputStyleArray = [InputStyle; 6];
    type InputContextData = KimeData;

    fn new_ic_data(
        &mut self,
        _server: &mut X11rbServer<C>,
        input_style: InputStyle,
    ) -> Result<Self::InputContextData, xim::ServerError> {
        let mut show_preedit_window = true;

        // Use callback instead
        if input_style.contains(InputStyle::PREEDIT_CALLBACKS) {
            show_preedit_window = false;
        }

        // Don't show preedit window on Xwayland see #137
        if !cfg!(debug_assertions)
            && std::env::var("XDG_SESSION_TYPE")
                .map(|v| v == "wayland")
                .unwrap_or(false)
        {
            show_preedit_window = false;
        }

        Ok(KimeData::new(
            &self.config,
            show_preedit_window,
            Arc::clone(&self.profile),
        ))
    }

    fn input_styles(&self) -> Self::InputStyleArray {
        [
            InputStyle::PREEDIT_NOTHING | InputStyle::STATUS_NOTHING,
            InputStyle::PREEDIT_POSITION | InputStyle::STATUS_NONE,
            InputStyle::PREEDIT_POSITION | InputStyle::STATUS_NOTHING,
            InputStyle::PREEDIT_POSITION | InputStyle::STATUS_CALLBACKS,
            InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_NOTHING,
            InputStyle::PREEDIT_CALLBACKS | InputStyle::STATUS_CALLBACKS,
        ]
    }

    fn filter_events(&self) -> u32 {
        EVENT_MASK
    }

    fn handle_connect(&mut self, _server: &mut X11rbServer<C>) -> Result<(), xim::ServerError> {
        Ok(())
    }

    fn handle_set_ic_values(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<KimeData>,
    ) -> Result<(), xim::ServerError> {
        log::debug!("spot: {:?}", user_ic.ic.preedit_spot());

        self.clear_preedit(server, user_ic)?;
        self.preedit(server, user_ic)?;

        Ok(())
    }

    fn handle_create_ic(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<KimeData>,
    ) -> Result<(), xim::ServerError> {
        log::info!(
            "IC created style: {:?}, spot_location: {:?}",
            user_ic.ic.input_style(),
            user_ic.ic.preedit_spot()
        );

        server.set_event_mask(&user_ic.ic, EVENT_MASK, 0)?;

        Ok(())
    }

    fn handle_reset_ic(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<Self::InputContextData>,
    ) -> Result<String, xim::ServerError> {
        log::trace!("reset_ic");
        self.stop_repeat(user_ic);
        self.reset(server, user_ic).map(|_| String::new())
    }

    fn handle_forward_event(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<Self::InputContextData>,
        xev: &KeyPressEvent,
    ) -> Result<bool, xim::ServerError> {
        let is_key_press = xev.response_type & !SEND_EVENT_FLAG == KEY_PRESS_EVENT;
        let repeat_id = Self::repeat_id(user_ic);

        // Releases are deliberately passed back to the client, but they must
        // also stop our fallback timer. XIM clients send both press and
        // release events because EVENT_MASK includes both.
        if !is_key_press {
            self.xim_repeats.remove(&repeat_id);
            return Ok(false);
        }

        log::trace!("{:?}", xev);

        // A real repeated press means this XIM client already implements
        // auto-repeat correctly. Stop the fallback so the two sources cannot
        // duplicate characters. A different real key starts a new repeat
        // candidate for this input context.
        let mut is_synthetic = false;
        let native_repeat = match self.xim_repeats.get(&repeat_id).copied() {
            Some(repeat) if repeat.keycode == xev.detail && repeat.pending > 0 => {
                if let Some(repeat) = self.xim_repeats.get_mut(&repeat_id) {
                    repeat.pending -= 1;
                }
                is_synthetic = true;
                false
            }
            Some(repeat) if repeat.keycode == xev.detail => {
                self.xim_repeats.remove(&repeat_id);
                true
            }
            Some(_) => {
                self.xim_repeats.remove(&repeat_id);
                false
            }
            None => false,
        };

        let mut state = ModifierState::empty();

        macro_rules! check_flag {
            ($mask:ident) => {
                (u16::from(xev.state) & u16::from(KeyButMask::$mask)) != 0
            };
        }

        if check_flag!(SHIFT) {
            state.insert(ModifierState::SHIFT);
        }

        if check_flag!(CONTROL) {
            state.insert(ModifierState::CONTROL);
        }

        if check_flag!(MOD1) {
            state.insert(ModifierState::ALT);
        }

        let numlock = check_flag!(MOD2);

        if check_flag!(MOD4) {
            state.insert(ModifierState::SUPER);
        }

        if let Some(keycode) = KeyCode::from_hardware_code(xev.detail as u16, numlock) {
            user_ic
                .user_data
                .engine
                .sync_profile(RuntimeProfile::from_u8(
                    user_ic.user_data.profile.load(Ordering::Relaxed),
                ));
            let ret = user_ic
                .user_data
                .engine
                .press_key(Key::new(keycode, state), &self.config);
            let consumed = self.process_input_result(server, user_ic, ret)?;

            if consumed
                && !is_synthetic
                && !native_repeat
                && self.can_repeat(Key::new(keycode, state))
            {
                self.start_repeat(user_ic, *xev);
            }

            Ok(consumed)
        } else {
            log::warn!("Unknown hardware keycode: {}", xev.detail);
            return Ok(false);
        }
    }

    fn handle_destroy_ic(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: xim::UserInputContext<Self::InputContextData>,
    ) -> Result<(), xim::ServerError> {
        log::info!("destroy_ic");

        self.xim_repeats.remove(&Self::repeat_id(&user_ic));

        if let Some(pe) = user_ic.user_data.pe {
            self.preedit_windows
                .remove(&pe)
                .unwrap()
                .clean(server.conn())?;
        }

        Ok(())
    }

    fn handle_set_focus(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<Self::InputContextData>,
    ) -> Result<(), xim::ServerError> {
        user_ic.user_data.engine.update_layout_state().ok();

        if !user_ic.user_data.engine_ready {
            if user_ic.user_data.engine.check_ready() {
                let ret = user_ic.user_data.engine.end_ready();
                self.process_input_result(server, user_ic, ret)?;
                user_ic.user_data.engine_ready = true;
            }
        }

        Ok(())
    }

    fn handle_unset_focus(
        &mut self,
        server: &mut X11rbServer<C>,
        user_ic: &mut xim::UserInputContext<Self::InputContextData>,
    ) -> Result<(), xim::ServerError> {
        self.stop_repeat(user_ic);
        if user_ic.user_data.engine_ready {
            self.reset(server, user_ic)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::KimeHandler;
    use kime_engine_core::Config;

    /// Regression test for #706: when the preedit font cannot be loaded
    /// (`load_font` returns empty data because no matching font is installed),
    /// constructing the handler must not panic. Instead the preedit window is
    /// disabled (`font == None`).
    #[test]
    fn missing_preedit_font_does_not_panic() {
        let mut config = Config::default();
        // Simulate a missing font: empty data, as produced by
        // `load_font(...).unwrap_or_default()` when no face matches.
        config.xim_preedit_font = (Vec::new(), 0, 15.0);

        let handler = KimeHandler::new(0, config);
        assert!(
            handler.font.is_none(),
            "preedit window should be disabled when the font is missing"
        );
    }
}
