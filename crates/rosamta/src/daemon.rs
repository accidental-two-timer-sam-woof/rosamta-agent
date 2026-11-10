//! `rsmt daemon run|enable|disable|status`.

use std::process::ExitCode;
use std::sync::mpsc;

use rosamta_core::apply::Context;
use rosamta_core::daemon::instance::{self, DaemonState, Instance};
use rosamta_core::daemon::service::{self, Service};
use rosamta_core::daemon::{self, Notifier, Stop, notify};
use rosamta_core::i18n::Lang;
use rosamta_core::paths::Paths;
use rosamta_core::{Result, adopt, sync, time};

use crate::print_json;

pub fn run() -> Result<ExitCode> {
    // Before any thread starts: the environment is changed in place. Paths
    // come after it, since the login shell may set `XDG_STATE_HOME`.
    let environment_error = daemon::adopt_login_environment();
    let ctx = Context::from_env()?;
    let state = ctx.paths.state().to_path_buf();
    let _instance = Instance::acquire(&state)?;
    let previous = instance::load(&state).ok().flatten().unwrap_or_default();
    instance::save(
        &state,
        &DaemonState {
            pid: std::process::id(),
            started_at: time::now(),
            version: env!("CARGO_PKG_VERSION").into(),
            environment_error,
            notified: previous.notified,
        },
    )?;
    let (events, receiver) = mpsc::channel();
    let lang = Lang::system();
    #[cfg(target_os = "macos")]
    if notify::in_app_bundle() {
        let notifier = notify::UserNotifications::set_up(lang, events.clone(), state);
        // Notification clicks arrive on the main thread's AppKit loop.
        std::thread::spawn(move || finish(daemon::run(ctx, Box::new(notifier), events, receiver)));
        notify::run_app();
    }
    #[cfg(target_os = "linux")]
    let notifier: Box<dyn Notifier> = match notify::Desktop::connect(lang, events.clone()) {
        Some(desktop) => Box::new(desktop),
        None => Box::new(notify::Silent),
    };
    #[cfg(not(target_os = "linux"))]
    let notifier: Box<dyn Notifier> = {
        let _ = lang;
        Box::new(notify::Silent)
    };
    finish(daemon::run(ctx, notifier, events, receiver))
}

/// Start the replaced executable in place of this process, or exit.
fn finish(stopped: Result<Stop>) -> ! {
    match stopped {
        Ok(Stop::Restart(exe)) => {
            use std::os::unix::process::CommandExt;
            let err = std::process::Command::new(&exe)
                .args(std::env::args_os().skip(1))
                .exec();
            eprintln!(
                "{}: cannot restart {}: {err}",
                crate::program_name(),
                exe.display()
            );
            std::process::exit(1)
        }
        Err(err) => {
            eprintln!("{}: {err}", crate::program_name());
            std::process::exit(1)
        }
    }
}

/// Homebrew links `rsmt` to the CLI inside rosamta.app, but macOS finds the
/// app bundle, which SMAppService needs, only when the program starts from
/// its real path. Start again from there.
fn from_app() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;
        const AGAIN: &str = "ROSAMTA_FROM_APP";
        if notify::in_app_bundle() || std::env::var_os(AGAIN).is_some() {
            return Ok(());
        }
        let Ok(exe) = std::env::current_exe().and_then(std::fs::canonicalize) else {
            return Ok(());
        };
        if !exe
            .parent()
            .is_some_and(|dir| dir.ends_with("Contents/MacOS"))
        {
            return Ok(());
        }
        let err = std::process::Command::new(&exe)
            .args(std::env::args_os().skip(1))
            .env(AGAIN, "1")
            .exec();
        return Err(rosamta_core::Error::Daemon(format!(
            "cannot start {}: {err}",
            exe.display()
        )));
    }
    #[allow(unreachable_code)]
    Ok(())
}

pub fn enable(json: bool) -> Result<ExitCode> {
    from_app()?;
    let paths = Paths::from_env()?;
    let service = service::enable(&paths)?;
    if json {
        print_json(&serde_json::json!({ "service": service }));
    } else if service == Service::RequiresApproval {
        println!("Allow rosamta in System Settings › General › Login Items to finish");
    } else {
        println!("The background service is enabled");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn disable(json: bool) -> Result<ExitCode> {
    from_app()?;
    let paths = Paths::from_env()?;
    let removed = service::disable(&paths)?;
    if json {
        print_json(&serde_json::json!({ "disabled": removed }));
    } else if removed {
        println!("The background service is disabled");
    } else {
        println!("The background service was not enabled");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn status(json: bool) -> Result<ExitCode> {
    from_app()?;
    let paths = Paths::from_env()?;
    let state = paths.state();
    let running = instance::running(state)?;
    let info = instance::load(state)?.filter(|_| running);
    let synced = sync::load_state(state)?;
    let review = Context::from_env()
        .and_then(|ctx| adopt::queue(&ctx))
        .map_or(0, |queue| queue.len());
    let service = service::status(&paths);
    if json {
        print_json(&serde_json::json!({
            "running": running,
            "pid": info.as_ref().map(|info| info.pid),
            "started_at": info.as_ref().map(|info| info.started_at),
            "version": info.as_ref().map(|info| &info.version),
            "environment_error": info.as_ref().and_then(|info| info.environment_error.as_ref()),
            "service": service,
            "publish": synced.publish,
            "pull": synced.pull,
            "unpushed": synced.unpushed,
            "review": review,
        }));
        return Ok(ExitCode::SUCCESS);
    }
    match &info {
        Some(info) => println!(
            "Running: pid {}, version {}, since {}",
            info.pid,
            info.version,
            time::date_time(info.started_at)
        ),
        None if running => println!("Running"),
        None => println!("Not running"),
    }
    if let Some(reason) = info
        .as_ref()
        .and_then(|info| info.environment_error.as_ref())
    {
        println!("Environment: not read from the login shell: {reason}");
    }
    let service = match service {
        Service::Enabled => "enabled",
        Service::RequiresApproval => "waiting for approval in System Settings",
        Service::NotRegistered => "not enabled",
        Service::Unavailable => "unavailable",
    };
    println!("Start at login: {service}");
    for (label, attempt) in [
        ("Last publish", &synced.publish),
        ("Last pull", &synced.pull),
    ] {
        if let Some(attempt) = attempt {
            let outcome = serde_json::to_value(attempt.outcome)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
            println!("{label}: {}, {outcome}", time::date_time(attempt.at));
        }
    }
    println!("To review: {review}");
    Ok(ExitCode::SUCCESS)
}
