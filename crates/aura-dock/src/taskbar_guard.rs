//! Start a watchdog only when it has confirmed that it can observe its owner.
//! Kept independent of Win32 so the startup/timeout protocol can be tested in CI.
use std::{io::{self, BufRead, BufReader}, process::{Child, Command, Stdio}, sync::mpsc, time::Duration};

pub const READY: &str = "AURA_DOCK_WATCHDOG_READY";

pub fn spawn_ready(command: &mut Command, timeout: Duration) -> io::Result<Child> {
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn()?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("watchdog stdout was not piped"));
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut input = BufReader::new(stdout);
        let mut line = String::new();
        let result = loop {
            line.clear();
            match input.read_line(&mut line) {
                Ok(0) => break Err(io::Error::other("watchdog exited without acknowledging startup")),
                Ok(_) if line.contains(READY) => break Ok(()),
                Ok(_) => continue,
                Err(error) => break Err(error),
            }
        };
        let _ = tx.send(result);
    });
    let result = match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(io::ErrorKind::TimedOut, "watchdog did not acknowledge startup")),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::other("watchdog readiness reader stopped")),
    };
    if let Err(error) = result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    // A process that acknowledged and exited is not a useful watchdog.
    match child.try_wait() {
        Ok(None) => Ok(child),
        Ok(Some(_)) => Err(io::Error::other("watchdog exited during startup")),
        Err(error) => { let _ = child.kill(); let _ = child.wait(); Err(error) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn helper_entry() {
        match std::env::var("AURA_GUARD_TEST").as_deref() {
            Ok("ready") => { println!("{}", READY); io::stdout().flush().unwrap(); std::thread::sleep(Duration::from_secs(20)); }
            Ok("silent") => std::thread::sleep(Duration::from_secs(20)),
            _ => {}
        }
    }
    fn helper(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        let module = module_path!().split_once("::").map(|(_, tail)| tail).unwrap_or(module_path!());
        command.arg("--exact").arg(format!("{module}::helper_entry")).arg("--nocapture").env("AURA_GUARD_TEST", mode);
        command
    }
    #[test]
    fn ready_guard_is_returned_alive() {
        let mut child = spawn_ready(&mut helper("ready"), Duration::from_secs(5)).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn silent_guard_is_killed_on_timeout() {
        assert_eq!(spawn_ready(&mut helper("silent"), Duration::from_millis(250)).unwrap_err().kind(), io::ErrorKind::TimedOut);
    }
    #[test]
    fn exited_guard_is_not_accepted() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.arg("--not-a-test-option");
        assert!(spawn_ready(&mut command, Duration::from_secs(5)).is_err());
    }
}
