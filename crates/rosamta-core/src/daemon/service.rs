//! Starting the daemon at login: a LaunchAgent inside rosamta.app,
//! registered through SMAppService on macOS (ADR 0004); a systemd user
//! service on Linux.

use serde::Serialize;

use crate::error::Result;
use crate::paths::Paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Service {
    Enabled,
    /// Registered, but waiting for the user to allow it in System Settings.
    RequiresApproval,
    NotRegistered,
    /// No way to register here: no rosamta.app, or no systemd user manager.
    Unavailable,
}

#[cfg(target_os = "macos")]
pub use macos::{disable, enable, status};

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::Retained;
    use objc2_foundation::{NSBundle, NSString};
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    use super::*;
    use crate::error::Error;

    const APP: &str = "tw.xinshou.rosamta";
    const PLIST: &str = "tw.xinshou.rosamta.agent.plist";

    /// SMAppService aborts the process outside an app bundle, so check that
    /// this executable is the one inside rosamta.app.
    fn agent() -> Option<Retained<SMAppService>> {
        let bundle = NSBundle::mainBundle().bundleIdentifier()?;
        (bundle.to_string() == APP)
            .then(|| unsafe { SMAppService::agentServiceWithPlistName(&NSString::from_str(PLIST)) })
    }

    fn from(status: SMAppServiceStatus) -> Service {
        match status {
            SMAppServiceStatus::Enabled => Service::Enabled,
            SMAppServiceStatus::RequiresApproval => Service::RequiresApproval,
            _ => Service::NotRegistered,
        }
    }

    pub fn status(_paths: &Paths) -> Service {
        agent().map_or(Service::Unavailable, |agent| {
            from(unsafe { agent.status() })
        })
    }

    pub fn enable(_paths: &Paths) -> Result<Service> {
        let agent = agent().ok_or_else(|| {
            Error::Usage(
                "on macOS the background service runs from rosamta.app; install the app first"
                    .into(),
            )
        })?;
        unsafe { agent.registerAndReturnError() }.map_err(|err| {
            Error::Daemon(format!(
                "cannot register the background service: {}",
                err.localizedDescription()
            ))
        })?;
        let status = from(unsafe { agent.status() });
        if status == Service::RequiresApproval {
            unsafe { SMAppService::openSystemSettingsLoginItems() };
        }
        Ok(status)
    }

    /// Returns whether it was registered.
    pub fn disable(_paths: &Paths) -> Result<bool> {
        let Some(agent) = agent() else {
            return Ok(false);
        };
        if from(unsafe { agent.status() }) == Service::NotRegistered {
            return Ok(false);
        }
        unsafe { agent.unregisterAndReturnError() }.map_err(|err| {
            Error::Daemon(format!(
                "cannot unregister the background service: {}",
                err.localizedDescription()
            ))
        })?;
        Ok(true)
    }
}

#[cfg(target_os = "linux")]
pub use linux::{disable, enable, status, unit_file};

#[cfg(target_os = "linux")]
mod linux {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;
    use crate::error::Error;

    const UNIT: &str = "rosamta.service";

    /// `$XDG_CONFIG_HOME/systemd/user/rosamta.service`, where systemd looks.
    pub fn unit_file(paths: &Paths) -> PathBuf {
        let config_home = paths
            .host_config()
            .parent()
            .and_then(Path::parent)
            .expect("host.toml lives two levels below the config home")
            .to_path_buf();
        config_home.join("systemd/user").join(UNIT)
    }

    fn systemctl(args: &[&str]) -> std::result::Result<String, String> {
        let output = Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .map_err(|err| err.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if output.status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if stderr.is_empty() { stdout } else { stderr })
        }
    }

    fn unit(exe: &Path) -> String {
        format!(
            "[Unit]\nDescription=rosamta background service\n\n[Service]\nExecStart=\"{}\" daemon run\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
            exe.display()
        )
    }

    pub fn status(_paths: &Paths) -> Service {
        // `is-enabled` exits non-zero for every state but enabled.
        match systemctl(&["is-enabled", UNIT]) {
            Ok(_) => Service::Enabled,
            Err(_) if systemctl(&["show-environment"]).is_err() => Service::Unavailable,
            Err(_) => Service::NotRegistered,
        }
    }

    pub fn enable(paths: &Paths) -> Result<Service> {
        if let Err(reason) = systemctl(&["show-environment"]) {
            return Err(Error::Daemon(format!(
                "no systemd user manager is running ({reason}); start the service in your session with `rsmt daemon run`"
            )));
        }
        let exe = std::env::current_exe().map_err(|err| Error::Read {
            path: "the rosamta executable".into(),
            source: err,
        })?;
        let file = unit_file(paths);
        let dir = file.parent().expect("the unit file has a parent");
        std::fs::create_dir_all(dir).map_err(Error::write(dir))?;
        crate::edit::write_atomic(&file, &unit(&exe))?;
        systemctl(&["daemon-reload"])
            .and_then(|_| systemctl(&["enable", "--now", UNIT]))
            .map_err(|reason| Error::Daemon(format!("cannot start {UNIT}: {reason}")))?;
        Ok(Service::Enabled)
    }

    /// Returns whether there was a unit to remove.
    pub fn disable(paths: &Paths) -> Result<bool> {
        let file = unit_file(paths);
        if !file.exists() {
            return Ok(false);
        }
        let _ = systemctl(&["disable", "--now", UNIT]);
        std::fs::remove_file(&file).map_err(Error::write(&file))?;
        let _ = systemctl(&["daemon-reload"]);
        Ok(true)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn unit_runs_this_executable() {
            let text = unit(Path::new("/usr/bin/rosamta"));
            assert!(text.contains("ExecStart=\"/usr/bin/rosamta\" daemon run\n"));
            assert!(text.contains("WantedBy=default.target"));
        }
    }
}
