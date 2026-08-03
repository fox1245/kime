use signal_hook::{
    consts::signal::{SIGUSR1, SIGUSR2},
    low_level,
};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use x11rb::{
    connection::Connection,
    protocol::{ErrorKind, Event},
};
use xim::{x11rb::HasConnection, XimConnections};

mod handler;
mod pe_window;

fn wait_for_x_event(fd: i32, timeout: Option<Duration>) -> bool {
    let timeout_ms = timeout
        .map(|duration| duration.as_millis().min(i32::MAX as u128) as i32)
        .unwrap_or(-1);
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };

    loop {
        // SAFETY: `pollfd` points to one initialized descriptor and remains
        // valid for the duration of this call.
        let result = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if result >= 0 {
            return result != 0;
        }

        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            panic!(
                "Poll X connection failed: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

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
    let x_fd = server.conn().stream().as_raw_fd();

    loop {
        // X11rb may already have decoded events in its internal queue while
        // the socket itself is idle. Drain that queue before polling the fd,
        // otherwise a pending key release could be delayed until the next
        // repeat tick.
        let e = if let Some(e) = server.conn().poll_for_event().expect("Poll event") {
            e
        } else {
            if !wait_for_x_event(x_fd, handler.repeat_timeout()) {
                if let Err(err) = handler.emit_repeats(server.conn()) {
                    log::error!("Failed to emit XIM key repeat: {}", err);
                    handler.clear_repeats();
                }
                server.conn().flush().expect("Flush connection");
                continue;
            }

            server.conn().wait_for_event().expect("Wait event")
        };

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
