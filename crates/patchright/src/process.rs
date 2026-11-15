//! Child-process setup shared by every place the crate spawns one.
//!
//! On Windows, `node.exe` (and `npm.cmd`, `where.exe`) are console-subsystem
//! programs. When the parent has no console of its own, as a GUI application
//! built with `#![windows_subsystem = "windows"]` does, Windows allocates a
//! fresh console for such a child and shows its window for as long as the
//! child lives. For the driver that is the whole session.
//!
//! `CREATE_NO_WINDOW` gives the child a console that is never shown. Its
//! stdio is already piped by every caller, so nothing is lost: the driver
//! speaks its protocol over the pipes and its stderr goes to `tracing`.
//! Processes the driver starts in turn (the browser, and anything the browser
//! starts) inherit that hidden console instead of allocating a visible one.
//! When the parent does have a console the flag changes nothing visible,
//! because the child would have shared it and its output is piped anyway.

/// `CREATE_NO_WINDOW` from `winbase.h`.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Keeps a spawned child from opening a console window on Windows.
/// A no-op elsewhere.
pub(crate) fn hide_console_std(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// [`hide_console_std`] for a tokio command.
pub(crate) fn hide_console(cmd: &mut tokio::process::Command) -> &mut tokio::process::Command {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
