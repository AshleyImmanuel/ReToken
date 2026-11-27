//! `retoken exec -- <command>`: runs a command and prints its output with the noise removed,
//! for agents whose model traffic cannot go through the proxy (for example the Antigravity
//! app). Short output is printed unchanged. Long output keeps its first and last lines and
//! every error, failure, warning and test-summary line; the full original is saved to the
//! local store and a marker tells the model how to fetch it with `retoken_retrieve`.

use anyhow::Context;
use compressors::ccr::store::MAX_AGE_SECS;
use compressors::ccr::CcrStore;
use compressors::tool_output::shorten_command_output;
use std::io::{Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};

/// The retrieval tool `retoken mcp` serves.
const RETRIEVE_TOOL: &str = "retoken_retrieve";

/// Runs `command`, prints its (possibly shortened) output and returns its exit code.
pub fn run_exec(command: Vec<String>) -> anyhow::Result<i32> {
    let (output, status) = run_captured(&command)?;
    let text = String::from_utf8_lossy(&output);
    let printed = CcrStore::open(None).ok().and_then(|store| {
        let short = shorten_command_output(&text, &store, RETRIEVE_TOOL);
        // Antigravity users may never run the proxy, whose hourly collection is otherwise the
        // only thing that removes stale originals. Collect only when this run stored one.
        if short.is_some() {
            store.gc(MAX_AGE_SECS);
        }
        short
    });
    let mut stdout = std::io::stdout().lock();
    match printed {
        Some(short) => writeln!(stdout, "{short}")?,
        None => stdout.write_all(&output)?,
    }
    stdout.flush()?;
    Ok(exit_code(status))
}

/// Runs the command with stdout and stderr captured into one buffer, in arrival order.
fn run_captured(command: &[String]) -> anyhow::Result<(Vec<u8>, ExitStatus)> {
    let Some((program, args)) = command.split_first() else {
        anyhow::bail!("exec requires a command (e.g. `retoken exec -- cargo test`)");
    };
    let mut child = spawn(program, args)?;
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|mut stream| {
        let buffer = Arc::clone(&buffer);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = stream.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buffer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend_from_slice(&chunk[..n]);
            }
        })
    })
    .collect();
    let status = child
        .wait()
        .context("Failed while waiting on the command")?;
    for reader in readers {
        let _ = reader.join();
    }
    let output = std::mem::take(&mut *buffer.lock().unwrap_or_else(|e| e.into_inner()));
    Ok((output, status))
}

fn spawn(program: &str, args: &[String]) -> anyhow::Result<std::process::Child> {
    let piped = |cmd: &mut Command| {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    };
    match piped(Command::new(program).args(args)) {
        Ok(child) => Ok(child),
        // Windows only finds `.exe` files directly; `npm`, `yarn` and other `.cmd` shims need
        // the command interpreter.
        #[cfg(windows)]
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            piped(Command::new("cmd").arg("/C").arg(program).args(args))
                .with_context(|| format!("Failed to run '{program}'. Is it installed and on PATH?"))
        }
        Err(e) => Err(e)
            .with_context(|| format!("Failed to run '{program}'. Is it installed and on PATH?")),
    }
}

/// The command's exit code; a process ended by a signal reports 128 + the signal number, as
/// shells do.
fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command that prints `lines` lines, one of them an error, then exits with `code`.
    fn noisy_command(lines: usize, code: i32) -> Vec<String> {
        let script = format!(
            "for i in range({lines}):\n    print('compiling module number', i, 'of the build')\n    if i == {mid}: print('error[E0425]: cannot find value `x` in this scope')\nraise SystemExit({code})",
            mid = lines / 2
        );
        vec!["python".into(), "-I".into(), "-c".into(), script]
    }

    fn python_available() -> bool {
        Command::new("python").arg("--version").output().is_ok()
    }

    #[test]
    fn captures_output_and_exit_code() {
        if !python_available() {
            return;
        }
        let (output, status) = run_captured(&noisy_command(5, 3)).unwrap();
        let text = String::from_utf8_lossy(&output);
        assert_eq!(text.lines().count(), 6);
        assert_eq!(exit_code(status), 3);
    }

    #[test]
    fn long_output_keeps_errors_and_stores_the_original() {
        if !python_available() {
            return;
        }
        let (output, _) = run_captured(&noisy_command(400, 1)).unwrap();
        let text = String::from_utf8_lossy(&output).into_owned();
        let store = CcrStore::open_in_memory().unwrap();
        let short =
            shorten_command_output(&text, &store, RETRIEVE_TOOL).expect("long output is shortened");
        assert!(short.len() < text.len() / 2);
        assert!(short.contains("error[E0425]: cannot find value `x` in this scope"));
        assert!(short.contains("lines elided (retoken)"));
        let handle = short
            .split("handle \"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("marker names a handle");
        let original = store.retrieve(handle).unwrap().expect("original is stored");
        assert_eq!(original, text.as_bytes());
    }

    #[test]
    fn short_output_is_left_alone() {
        let store = CcrStore::open_in_memory().unwrap();
        assert!(shorten_command_output("ok\n", &store, RETRIEVE_TOOL).is_none());
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn missing_program_fails() {
        let result = run_captured(&["retoken-no-such-program-xyz".to_string()]);
        // On Windows the `cmd /C` fallback starts, then reports the unknown command.
        #[cfg(windows)]
        assert_ne!(exit_code(result.unwrap().1), 0);
        #[cfg(not(windows))]
        assert!(result.is_err());
    }

    #[test]
    fn empty_command_is_an_error() {
        assert!(run_captured(&[]).is_err());
    }
}
