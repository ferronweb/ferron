//! Unix daemon module for running the web server as a background service.
//!
//! This module provides functionality to daemonize the process using `fork` and `setsid`,
//! manage PID files, and handle Unix signals (SIGINT, SIGHUP) for graceful shutdown and reload.

#![cfg(unix)]

use std::fs::{self, File};
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use ferron_core::{log_debug, log_info};

/// Daemonize the current process by forking and creating a new session.
///
/// This function performs the following steps:
/// 1. Fork the process
/// 2. Exit the parent process
/// 3. Create a new session with `setsid`
/// 4. Change working directory to root
/// 5. Set umask to 0
/// 6. Close standard file descriptors
///
/// Returns `true` if this is the child (daemon) process, `false` if this is the parent.
pub fn daemonize() -> Result<bool> {
    use nix::unistd;

    // First fork
    match unsafe { unistd::fork()? } {
        unistd::ForkResult::Parent { child } => {
            // Parent process: wait for child and exit
            log_debug!(
                "Parent process (PID {}) forking daemon, child PID: {}",
                unistd::getpid(),
                child
            );
            return Ok(false);
        }
        unistd::ForkResult::Child => {
            // Child process: continue to daemonize
        }
    }

    // Create a new session and become the session leader
    unistd::setsid().context("Failed to create new session")?;
    log_debug!("Created new session, SID: {}", unistd::getsid(None)?);

    // Second fork to ensure we can never acquire a controlling terminal
    match unsafe { unistd::fork()? } {
        unistd::ForkResult::Parent { child } => {
            // First child exits
            log_debug!(
                "First child (PID {}) exiting, daemon PID: {}",
                unistd::getpid(),
                child
            );
            std::process::exit(0);
        }
        unistd::ForkResult::Child => {
            // Grandchild: the actual daemon
        }
    }

    // Change working directory to root to avoid keeping any directory in use
    unistd::chdir("/").context("Failed to change directory to /")?;

    // Set umask to 0 to have full control over file permissions
    use nix::sys::stat::Mode;
    nix::sys::stat::umask(Mode::from_bits_truncate(0));

    // Close standard file descriptors (stdin, stdout, stderr)
    // They will be redirected to /dev/null
    close_standard_fds()?;

    log_info!("Daemon process started, PID: {}", unistd::getpid());

    Ok(true)
}

/// Close standard file descriptors and redirect them to /dev/null
fn close_standard_fds() -> Result<()> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;
    use nix::unistd::{close, dup2};

    // Open /dev/null
    let devnull =
        open("/dev/null", OFlag::O_RDWR, Mode::empty()).context("Failed to open /dev/null")?;

    // Redirect stdin, stdout, stderr to /dev/null
    dup2(&devnull, &mut (unsafe { OwnedFd::from_raw_fd(0) }))
        .context("Failed to redirect stdin")?;
    dup2(&devnull, &mut (unsafe { OwnedFd::from_raw_fd(1) }))
        .context("Failed to redirect stdout")?;
    dup2(&devnull, &mut (unsafe { OwnedFd::from_raw_fd(2) }))
        .context("Failed to redirect stderr")?;

    // Close the original fd if it's not 0, 1, or 2
    if devnull.as_raw_fd() > 2 {
        close(devnull).ok();
    }

    Ok(())
}

/// Write the current process ID to a PID file
pub fn write_pid_file(path: &str) -> Result<()> {
    use nix::unistd::getpid;

    let pid = getpid().as_raw();
    let pid_path = Path::new(path);

    // Create parent directories if they don't exist
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)
            .context(format!("Failed to create PID file directory: {}", path))?;
    }

    let mut file =
        File::create(pid_path).context(format!("Failed to create PID file: {}", path))?;

    writeln!(file, "{}", pid).context(format!("Failed to write PID to file: {}", path))?;

    file.flush()
        .context(format!("Failed to flush PID file: {}", path))?;

    log_info!("PID file written: {} (PID: {})", path, pid);

    Ok(())
}

/// Remove the PID file
pub fn remove_pid_file(path: &str) -> Result<()> {
    fs::remove_file(path).context(format!("Failed to remove PID file: {}", path))?;

    log_debug!("PID file removed: {}", path);

    Ok(())
}

/// Check if a PID file exists and contains a valid PID of a running process
pub fn check_pid_file(path: &str) -> Result<bool> {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let pid_path = Path::new(path);

    if !pid_path.exists() {
        return Ok(false);
    }

    let content =
        fs::read_to_string(pid_path).context(format!("Failed to read PID file: {}", path))?;

    let pid: i32 = content
        .trim()
        .parse()
        .context(format!("Invalid PID in file: {}", path))?;

    // Check if the process is running by sending signal 0
    match kill(Pid::from_raw(pid), None) {
        Ok(_) => Ok(true),
        Err(nix::errno::Errno::ESRCH) => Ok(false), // Process doesn't exist
        Err(e) => Err(anyhow::anyhow!("Failed to check process: {}", e)),
    }
}

/// Set up Unix signal handlers for graceful shutdown and reload.
///
/// This function spawns a thread that listens for SIGINT and SIGHUP signals:
/// - SIGINT triggers a graceful shutdown (cancels SHUTDOWN_TOKEN)
/// - SIGHUP triggers a configuration reload (cancels RELOAD_TOKEN)
pub fn setup_signal_handlers() -> Result<()> {
    use ferron_core::shutdown::{RELOAD_TOKEN, SHUTDOWN_TOKEN};
    use signal_hook::consts::signal::{SIGHUP, SIGINT};
    use signal_hook::iterator::Signals;

    let mut signals = Signals::new([SIGINT, SIGHUP])
        .context("Failed to set up signal handlers for SIGINT and SIGHUP")?;
    std::thread::spawn(move || {
        for signal in signals.forever() {
            match signal {
                SIGINT => {
                    log_debug!("Received SIGINT, initiating graceful shutdown");
                    SHUTDOWN_TOKEN
                        .swap(Arc::new(tokio_util::sync::CancellationToken::new()))
                        .cancel();
                }
                SIGHUP => {
                    log_debug!("Received SIGHUP, initiating configuration reload");
                    RELOAD_TOKEN
                        .swap(Arc::new(tokio_util::sync::CancellationToken::new()))
                        .cancel();
                }
                _ => unreachable!(),
            }
        }
    });

    log_debug!("Signal handlers installed for SIGINT and SIGHUP");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pid_file_operations() {
        let temp_dir = std::env::temp_dir();
        let pid_file = temp_dir.join("ferron_test.pid");
        let pid_path = pid_file.to_string_lossy();

        // Clean up any existing file
        let _ = fs::remove_file(&pid_file);

        // Write PID file
        write_pid_file(&pid_path).unwrap();

        // Check PID file exists
        assert!(pid_file.exists());

        // Check PID file content
        let running = check_pid_file(&pid_path).unwrap();
        assert!(running);

        // Remove PID file
        remove_pid_file(&pid_path).unwrap();
        assert!(!pid_file.exists());
    }
}
