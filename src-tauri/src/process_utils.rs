#[cfg(any(target_os = "windows", test))]
pub(crate) const WINDOWS_CREATE_NO_WINDOW_FLAG: u32 = 0x08000000;

#[cfg(test)]
pub(crate) fn windows_create_no_window_flag() -> u32 {
    WINDOWS_CREATE_NO_WINDOW_FLAG
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn hide_std_command_window(command: &mut std::process::Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(WINDOWS_CREATE_NO_WINDOW_FLAG);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

pub(crate) fn hide_tokio_command_window(command: &mut tokio::process::Command) {
    #[cfg(target_os = "windows")]
    {
        use tokio::os::windows::process::CommandExt;
        command.creation_flags(WINDOWS_CREATE_NO_WINDOW_FLAG);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}
