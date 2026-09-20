use std::process::Command;

/// Arrange for a spawned worker to receive SIGTERM if the desktop process dies.
///
/// Linux clears PR_SET_PDEATHSIG across fork, so it must be installed in the
/// child immediately before exec. Checking the parent PID afterwards closes the
/// race where the desktop exits between fork and the prctl call.
#[cfg(target_os = "linux")]
pub fn terminate_with_parent(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    let expected_parent = std::process::id() as libc::pid_t;
    // SAFETY: pre_exec runs after fork and before exec. The closure only calls
    // async-signal-safe libc functions and constructs an error on prctl failure.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != expected_parent {
                libc::raise(libc::SIGTERM);
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
pub fn terminate_with_parent(_command: &mut Command) {}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{
        io::Write,
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };

    const HELPER_ENV: &str = "TRANSCRIP_IT_PDEATHSIG_TEST_HELPER";
    const CHILD_MARKER: &str = "PDEATHSIG_CHILD=";

    #[test]
    // The helper must deliberately exit without waiting so the test can verify
    // that the kernel terminates and reaps the protected worker independently.
    #[allow(clippy::zombie_processes)]
    fn worker_terminates_when_its_parent_exits() {
        if std::env::var_os(HELPER_ENV).is_some() {
            let mut worker = Command::new("sleep");
            worker.arg("30");
            terminate_with_parent(&mut worker);
            let child = worker.spawn().expect("spawn protected worker");
            println!("{CHILD_MARKER}{}", child.id());
            std::io::stdout().flush().expect("flush child pid");
            return;
        }

        let output = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "child_process::tests::worker_terminates_when_its_parent_exits",
                "--nocapture",
            ])
            .env(HELPER_ENV, "1")
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .output()
            .expect("run parent helper");
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).expect("utf8 helper output");
        let pid = stdout
            .lines()
            .find_map(|line| line.strip_prefix(CHILD_MARKER))
            .expect("protected child pid")
            .parse::<u32>()
            .expect("numeric child pid");

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && !process_has_terminated(pid) {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            process_has_terminated(pid),
            "worker {pid} survived its parent process"
        );
    }

    fn process_has_terminated(pid: u32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return true;
        };
        stat.rsplit_once(") ")
            .and_then(|(_, fields)| fields.chars().next())
            .is_some_and(|state| state == 'Z')
    }
}
