use signal_hook::{
    consts::signal::{SIGUSR1, SIGUSR2},
    low_level,
};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use x11rb::{
    connection::Connection,
    protocol::{ErrorKind, Event},
};
use xim::{x11rb::HasConnection, XimConnections};

mod handler;
mod pe_window;

fn register_profile_signals(profile: &Arc<AtomicU8>) {
    let game_profile = Arc::clone(profile);
    let normal_profile = Arc::clone(profile);

    unsafe {
        low_level::register(
            SIGUSR1,
            Box::new(move || {
                game_profile.store(
                    kime_engine_core::RuntimeProfile::Game.as_u8(),
                    Ordering::Relaxed,
                );
            }),
        )
        .expect("Register game profile signal");
        low_level::register(
            SIGUSR2,
            Box::new(move || {
                normal_profile.store(
                    kime_engine_core::RuntimeProfile::Normal.as_u8(),
                    Ordering::Relaxed,
                );
            }),
        )
        .expect("Register normal profile signal");
    }
}

fn main() {
    kime_version::cli_boilerplate!((),);

    let config = kime_engine_core::load_engine_config_from_config_dir().unwrap_or_default();

    let (conn, screen_num) =
        x11rb::rust_connection::RustConnection::connect(None).expect("Connect X");
    let profile = Arc::new(AtomicU8::new(
        kime_engine_core::RuntimeProfile::Normal.as_u8(),
    ));
    register_profile_signals(&profile);
    let mut server = xim::x11rb::X11rbServer::init(conn, screen_num, "kime", xim::ALL_LOCALES)
        .expect("Init XIM server");
    let mut connections = XimConnections::new();
    let mut handler = self::handler::KimeHandler::with_profile(screen_num, config, profile);

    loop {
        let e = server.conn().wait_for_event().expect("Wait event");
        match server.filter_event(&e, &mut connections, &mut handler) {
            // event has filtered
            Ok(true) => {}
            // event hasn't filtered
            Ok(false) => match e {
                Event::Expose(e) => {
                    handler.expose(e.window, server.conn()).unwrap();
                    server.conn().flush().expect("Flush connection");
                }
                Event::ConfigureNotify(e) => {
                    handler.configure_notify(e, server.conn()).unwrap();
                    server.conn().flush().expect("Flush connection");
                }
                Event::UnmapNotify(..) => {}
                Event::DestroyNotify(..) => {}
                Event::MappingNotify(..) => {}
                Event::Error(x11rb::x11_utils::X11Error {
                    error_kind: ErrorKind::RenderPicture,
                    ..
                }) => {}
                e => {
                    log::trace!("Unfiltered event: {:?}", e);
                }
            },
            Err(err) => {
                // Don't stop server just logging
                log::error!("ServerError occurred while process event: {}", err);
            }
        }
    }
}
