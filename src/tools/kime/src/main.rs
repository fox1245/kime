use kime_engine_core::{load_raw_config_from_config_dir, DaemonModule as Module};
use nix::fcntl::{Flock, FlockArg};
use nix::sys::signal::{kill, Signal};
use nix::unistd::{daemon, Pid};
use signal_hook::{
    consts::signal::{SIGUSR1, SIGUSR2},
    flag,
};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{fs::File, io::Write, path::Path};
use std::{
    io,
    process::{Command, Stdio},
};

const fn process_name(module: Module) -> &'static str {
    match module {
        Module::Xim => "kime-xim",
        Module::Wayland => "kime-wayland",
        Module::Indicator => "kime-indicator",
    }
}

fn signal_daemon(pid_path: &Path, signal: Signal) -> io::Result<()> {
    let pid_str = std::fs::read_to_string(pid_path)?;
    let pid: i32 = match pid_str.trim().parse() {
        Ok(pid) => pid,
        Err(err) => {
            log::error!("kill return: {}", err);
            return Err(io::Error::new(io::ErrorKind::Other, "kill command failed"));
        }
    };

    match kill(Pid::from_raw(pid), signal) {
        Ok(_) => Ok(()),
        Err(err) => {
            log::error!("kill return: {}", err);
            Err(io::Error::new(io::ErrorKind::Other, "kill command failed"))
        }
    }
}

fn requested_profile_signal(args: &[String]) -> Result<Option<Signal>, String> {
    for (index, arg) in args.iter().enumerate() {
        let value = if let Some(value) = arg.strip_prefix("--profile=") {
            Some(value.to_string())
        } else if arg == "--profile" {
            Some(
                args.get(index + 1)
                    .ok_or_else(|| "--profile requires normal or game".to_string())?
                    .clone(),
            )
        } else {
            None
        };

        if let Some(value) = value {
            return match value.as_str() {
                "normal" => Ok(Some(Signal::SIGUSR2)),
                "game" => Ok(Some(Signal::SIGUSR1)),
                _ => Err(format!("unknown profile {:?}; use normal or game", value)),
            };
        }
    }

    Ok(None)
}

fn send_profile_to_children(
    processes: &mut [(&'static str, std::process::Child, bool)],
    signal: Signal,
) {
    for (name, process, exited) in processes {
        if *exited {
            continue;
        }

        if let Err(err) = kill(Pid::from_raw(process.id() as i32), signal) {
            log::warn!("Can't send {:?} to {}: {}", signal, name, err);
        }
    }
}

fn main() -> Result<(), ()> {
    let raw_args = std::env::args().collect::<Vec<_>>();
    let mut args = kime_version::cli_boilerplate!(
        Ok(()),
        "-k or --kill: kill daemon then exit",
        "-D or --no-daemon: don't start as daemon",
        "--profile <normal|game>: switch runtime hotkey profile",
    );

    let run_dir = kime_run_dir::get_run_dir();
    let pid = run_dir.join("kime.pid");

    match requested_profile_signal(&raw_args) {
        Ok(Some(signal)) => {
            return signal_daemon(&pid, signal).map_err(|err| {
                log::error!("Can't switch Kime profile: {}", err);
            });
        }
        Ok(None) => {}
        Err(err) => {
            log::error!("{}", err);
            return Err(());
        }
    }

    if args.contains(["-k", "--kill"]) {
        return signal_daemon(&pid, Signal::SIGTERM).map_err(|err| {
            log::error!("Can't kill daemon: {}", err);
        });
    }

    if !args.contains(["-D", "--no-daemon"]) {
        let stderr_path = run_dir.join("kime.err");
        let stderr_file = match File::create(&stderr_path) {
            Ok(file) => file,
            Err(err) => {
                log::error!("Can't create stderr file: {}", err);
                return Err(());
            }
        };

        // Daemonize: fork and detach from terminal (noclose=true to keep fds open)
        match daemon(true, true) {
            Ok(_) => {}
            Err(err) => {
                log::error!("Can't daemonize kime: {}", err);
                return Err(());
            }
        }

        // Change working directory to /tmp
        let _ = std::env::set_current_dir("/tmp");

        // Redirect stderr to file
        unsafe {
            nix::libc::dup2(stderr_file.as_raw_fd(), nix::libc::STDERR_FILENO);
        }
    }

    // Create PID file and lock it exclusively to prevent duplicate instances
    // Lock must be held until program exits (like daemonize library behavior)
    // Possible errors:
    //   - File::create fails: permission denied, disk full, etc.
    //   - Flock::lock returns EWOULDBLOCK: another kime instance is running
    //   - Flock::lock returns other error: unexpected lock failure
    let _pid_lock = match File::create(&pid).and_then(|file| {
        Flock::lock(file, FlockArg::LockExclusiveNonblock).map_err(|(_, e)| io::Error::from(e))
    }) {
        Ok(mut lock) => {
            writeln!(lock, "{}", std::process::id()).map_err(|err| {
                log::error!("Can't daemonize kime: {}", err);
            })?;
            Some(lock)
        }
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
            log::error!("Another instance of kime daemon is already running.");
            return Err(());
        }
        Err(err) => {
            log::error!("Can't daemonize kime: {}", err);
            return Err(());
        }
    };

    let config = load_raw_config_from_config_dir().daemon;

    static RUN: AtomicBool = AtomicBool::new(true);

    let game_profile = Arc::new(AtomicBool::new(false));
    let normal_profile = Arc::new(AtomicBool::new(false));
    flag::register(SIGUSR1, Arc::clone(&game_profile)).expect("Register game profile signal");
    flag::register(SIGUSR2, Arc::clone(&normal_profile)).expect("Register normal profile signal");

    ctrlc::set_handler(|| {
        log::info!("Receive exit signal");
        RUN.store(false, Ordering::SeqCst);
    })
    .expect("Set ctrlc handler");

    log::info!("Initialized");

    let mut processes = config
        .modules
        .iter()
        .filter_map(|module| {
            let name = process_name(module);
            match Command::new(name)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
            {
                Ok(p) => Some((name, p, false)),
                Err(err) => {
                    log::error!("Can't spawn {}: {}", name, err);
                    None
                }
            }
        })
        .collect::<Vec<_>>();

    while RUN.load(Ordering::SeqCst) {
        if game_profile.swap(false, Ordering::SeqCst) {
            send_profile_to_children(&mut processes, Signal::SIGUSR1);
        }
        if normal_profile.swap(false, Ordering::SeqCst) {
            send_profile_to_children(&mut processes, Signal::SIGUSR2);
        }

        // Remove finished process
        for (name, process, exited) in processes.iter_mut() {
            match process.try_wait().expect("Wait process") {
                Some(status) => {
                    log::info!("Process {} has exit with {}", name, status);
                    *exited = true;
                }
                None => {}
            }
        }

        processes.retain(|(_, _, exited)| !*exited);

        if processes.is_empty() {
            log::info!("All process has exited");
            return Ok(());
        }

        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    for (name, mut process, _) in processes {
        log::info!("KILL {}", name);
        process.kill().ok();
    }

    Ok(())
}
