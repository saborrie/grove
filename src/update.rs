//! `grove update` — replace this binary with the newest release.
//!
//! The installer is baked in with `include_str!` rather than fetched. That keeps
//! one implementation of download-verify-replace, versioned alongside the binary
//! that runs it, and it means grove never pulls a script off the network and
//! executes it on its own initiative — piping the installer into `sh` is a thing
//! you choose to do, not something a file manager does behind your back.
//!
//! The trade is that a very old grove carries a very old installer. If the shape
//! of a release ever changes, `grove update` from before that change cannot
//! follow it, and the install script has to be re-run by hand. That is the same
//! position anyone who has never run `grove update` is already in.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const INSTALLER: &str = include_str!("../scripts/install.sh");

/// What an update should do, given what is installed and what is released.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// A build from a checkout has no release behind it, and replacing it with
    /// one would throw away whatever was being worked on.
    RefuseSourceBuild,
    AlreadyCurrent,
    Install,
}

fn decide(current: &str, latest: &str) -> Action {
    match current {
        "dev" => Action::RefuseSourceBuild,
        _ if current == latest => Action::AlreadyCurrent,
        _ => Action::Install,
    }
}

pub fn run(check_only: bool) -> Result<(), String> {
    let current = crate::VERSION;
    let latest = latest_release()?;

    if check_only {
        println!("installed: grove {current}");
        println!("latest:    grove {latest}");
        if decide(current, &latest) == Action::Install {
            println!("\nrun `grove update` to install it");
        }
        return Ok(());
    }

    match decide(current, &latest) {
        Action::RefuseSourceBuild => Err(format!(
            "this is a build from a checkout, not a release — it reports no version, \
             so there is nothing to compare against {latest}.\n\
             Install the release over it deliberately if that is what you want:\n    \
             curl -fsSL https://raw.githubusercontent.com/saborrie/grove/main/scripts/install.sh | sh"
        )),
        Action::AlreadyCurrent => {
            println!("grove {current} is the latest release");
            Ok(())
        }
        Action::Install => {
            let dir = install_dir()?;
            // Replacing the binary means creating a file next to it and renaming
            // over it, so it is the directory that has to be writable, not the
            // file. Say so before the download rather than after it.
            if !writable(&dir) {
                return Err(format!(
                    "cannot write to {} — try:\n    sudo GROVE_INSTALL_DIR={} sh -c \
                     \"$(curl -fsSL https://raw.githubusercontent.com/saborrie/grove/main/scripts/install.sh)\"",
                    dir.display(),
                    dir.display()
                ));
            }
            installer(&[("GROVE_INSTALL_DIR", &dir.to_string_lossy())], false)?;
            Ok(())
        }
    }
}

/// The newest released version, straight from the installer so that there is
/// only one thing that knows how to work it out.
fn latest_release() -> Result<String, String> {
    let out = installer(&[("GROVE_CHECK", "1")], true)?;
    let latest = out.trim().to_string();
    if latest.is_empty() {
        return Err("could not work out the latest version".into());
    }
    Ok(latest)
}

/// The directory holding the binary that is running — not a guess at where it
/// ought to live. Someone who put grove in /usr/local/bin means to update the
/// one in /usr/local/bin.
fn install_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this binary: {e}"))?;
    // Through a symlink, update what it points at rather than replacing the link.
    let exe = exe.canonicalize().unwrap_or(exe);
    exe.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} has no directory to install into", exe.display()))
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".grove-update-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Run the embedded installer under `sh`, with `vars` set. Output is captured
/// when we need an answer back and inherited when the user wants to watch it
/// work; either way stderr goes straight to the terminal so failures are seen.
fn installer(vars: &[(&str, &str)], capture: bool) -> Result<String, String> {
    let mut command = Command::new("sh");
    command.arg("-s").stdin(Stdio::piped());
    for (key, value) in vars {
        command.env(key, value);
    }
    if capture {
        command.stdout(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not run sh: {e}"))?;
    // Dropped at the end of the statement, which closes the pipe and lets `sh`
    // reach end of input — without that it would wait for more script forever.
    child
        .stdin
        .take()
        .ok_or("sh took no stdin")?
        .write_all(INSTALLER.as_bytes())
        .map_err(|e| format!("could not hand the installer to sh: {e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("installer did not finish: {e}"))?;
    if !output.status.success() {
        return Err("the installer failed — see above".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_release_is_installed() {
        assert_eq!(decide("0.3.0", "0.3.1"), Action::Install);
    }

    #[test]
    fn the_same_version_is_left_alone() {
        // Re-downloading what you already have is only slow, not wrong — but
        // saying so is friendlier than two megabytes of nothing.
        assert_eq!(decide("0.3.1", "0.3.1"), Action::AlreadyCurrent);
    }

    #[test]
    fn a_source_build_is_never_replaced_silently() {
        // `dev` is what a build from a checkout reports. Overwriting it with a
        // release would quietly throw away whatever was being worked on.
        assert_eq!(decide("dev", "0.3.1"), Action::RefuseSourceBuild);
        assert_eq!(decide("dev", "dev"), Action::RefuseSourceBuild);
    }

    #[test]
    fn the_installer_is_embedded_whole() {
        // include_str! of a path that moved would still compile as an empty or
        // wrong file; check this is the real script before shipping it to sh.
        assert!(INSTALLER.starts_with("#!/bin/sh"));
        assert!(
            INSTALLER.contains("GROVE_CHECK"),
            "check mode must be present"
        );
        assert!(
            INSTALLER.trim_end().ends_with(r#"main "$@""#),
            "the script must still run itself"
        );
    }

    #[test]
    fn the_running_binary_is_what_gets_updated() {
        let dir = install_dir().unwrap();
        assert!(dir.is_dir());
        assert!(
            std::env::current_exe().unwrap().starts_with(&dir),
            "the install directory must contain this binary"
        );
    }
}
