//! Autostart through a per-user Task Scheduler logon task.
//!
//! Not the `Run` registry key: Explorer runs those after its startup delay and in
//! a batch with every other tray app. A logon trigger fires as soon as the user
//! session exists, without a delay, and the task sets normal CPU/IO priority
//! (the Task Scheduler default of 7 means below-normal).
//!
//! COM calls take tens of milliseconds, so callers run them off the UI thread.

use windows::Win32::Security::Authentication::Identity::{GetUserNameExW, NameSamCompatible};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::TaskScheduler::{
    ITaskFolder, ITaskService, TASK_CREATE_OR_UPDATE, TASK_LOGON_INTERACTIVE_TOKEN, TaskScheduler,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, PWSTR, Result};

/// Command-line flag the task passes, so the app can tell an autostart launch.
pub const FLAG: &str = "--autostart";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Off,
    On { command: String, current_exe: bool },
}

fn user_name() -> Option<String> {
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    unsafe { GetUserNameExW(NameSamCompatible, Some(PWSTR(buf.as_mut_ptr())), &mut len) }
        .then(|| String::from_utf16_lossy(&buf[..len as usize]))
}

fn task_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    format!("Ambient Notes autostart ({user})")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn task_xml(user: &str, exe: &std::path::Path) -> String {
    let user = xml_escape(user);
    let command = xml_escape(&exe.display().to_string());
    let dir = xml_escape(&exe.parent().map(|p| p.display().to_string()).unwrap_or_default());
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Author>{user}</Author>
    <Description>Starts Ambient Notes when {user} signs in.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{command}"</Command>
      <Arguments>{FLAG}</Arguments>
      <WorkingDirectory>{dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// Runs `f` with a connected Task Scheduler root folder on a COM-initialized thread.
fn with_root<T>(f: impl FnOnce(&ITaskFolder) -> Result<T>) -> Result<T> {
    unsafe {
        let init = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = (|| {
            let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)?;
            let empty = VARIANT::default();
            service.Connect(&empty, &empty, &empty, &empty)?;
            let root = service.GetFolder(&BSTR::from("\\"))?;
            f(&root)
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        result
    }
}

fn command_from_xml(xml: &str) -> Option<String> {
    let start = xml.find("<Command>")? + "<Command>".len();
    let end = start + xml[start..].find("</Command>")?;
    Some(xml[start..end].trim().trim_matches('"').replace("&amp;", "&"))
}

pub fn status() -> Result<Status> {
    with_root(|root| unsafe {
        let Ok(task) = root.GetTask(&BSTR::from(task_name())) else {
            return Ok(Status::Off);
        };
        if !task.Enabled()?.as_bool() {
            return Ok(Status::Off);
        }
        let command = command_from_xml(&task.Xml()?.to_string()).unwrap_or_default();
        let current_exe = std::env::current_exe()
            .is_ok_and(|exe| exe.as_os_str().eq_ignore_ascii_case(std::ffi::OsStr::new(&command)));
        Ok(Status::On { command, current_exe })
    })
}

/// Registers (or re-points to the current executable) the logon task.
pub fn enable() -> Result<()> {
    let exe = std::env::current_exe().map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
    let user = user_name().ok_or_else(windows::core::Error::from_thread)?;
    let xml = task_xml(&user, &exe);
    with_root(|root| unsafe {
        let empty = VARIANT::default();
        root.RegisterTask(
            &BSTR::from(task_name()),
            &BSTR::from(xml),
            TASK_CREATE_OR_UPDATE.0,
            &empty,
            &empty,
            TASK_LOGON_INTERACTIVE_TOKEN,
            &empty,
        )?;
        Ok(())
    })
}

pub fn disable() -> Result<()> {
    with_root(|root| unsafe {
        match root.DeleteTask(&BSTR::from(task_name()), 0) {
            // ERROR_FILE_NOT_FOUND: already gone.
            Err(e) if e.code() == windows::core::HRESULT::from_win32(2) => Ok(()),
            other => other,
        }
    })
}

/// Starts the task now, the same way a logon would (for testing).
pub fn run_now() -> Result<()> {
    with_root(|root| unsafe {
        root.GetTask(&BSTR::from(task_name()))?.Run(&VARIANT::default())?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_round_trips_command() {
        let xml = task_xml(r"PC\me", std::path::Path::new(r"C:\A & B\ambient.exe"));
        assert_eq!(command_from_xml(&xml).as_deref(), Some(r"C:\A & B\ambient.exe"));
        assert!(xml.contains("<Priority>4</Priority>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
    }
}
