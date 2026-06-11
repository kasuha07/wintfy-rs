use std::{
    fs,
    mem::ManuallyDrop,
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        System::{
            Com::StructuredStorage::{
                PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0, PropVariantClear,
            },
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemAlloc, CoUninitialize, IPersistFile,
            },
            Variant::{VARENUM, VT_LPWSTR},
        },
        UI::Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink},
    },
    core::{GUID, Interface, PCWSTR},
};

use crate::platform::paths;

const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};

pub fn open_path(path: &Path) -> Result<(), String> {
    Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    Ok(())
}

pub fn create_shortcut(
    shortcut: &Path,
    target: &Path,
    args: Option<&str>,
    aumid: &str,
) -> Result<(), String> {
    if let Some(parent) = shortcut.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create shortcut directory {}: {err}",
                parent.display()
            )
        })?;
    }
    let _com = ComGuard::init()?;
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|err| format!("CoCreateInstance ShellLink failed: {err}"))?;
        let target_w = wide_path(target);
        link.SetPath(PCWSTR(target_w.as_ptr()))
            .map_err(|err| format!("IShellLink SetPath failed: {err}"))?;
        if let Some(args) = args {
            let args_w = wide(args);
            link.SetArguments(PCWSTR(args_w.as_ptr()))
                .map_err(|err| format!("IShellLink SetArguments failed: {err}"))?;
        }
        if let Some(workdir) = target.parent() {
            let workdir_w = wide_path(workdir);
            link.SetWorkingDirectory(PCWSTR(workdir_w.as_ptr()))
                .map_err(|err| format!("IShellLink SetWorkingDirectory failed: {err}"))?;
        }

        let store: IPropertyStore = link
            .cast()
            .map_err(|err| format!("IPropertyStore cast failed: {err}"))?;
        let mut prop = propvariant_lpwstr(aumid)?;
        if let Err(err) = store.SetValue(&PKEY_APP_USER_MODEL_ID, &prop) {
            let _ = PropVariantClear(&mut prop);
            return Err(format!("setting AUMID failed: {err}"));
        }
        if let Err(err) = store.Commit() {
            let _ = PropVariantClear(&mut prop);
            return Err(format!("shortcut property commit failed: {err}"));
        }
        PropVariantClear(&mut prop)
            .map_err(|err| format!("clearing shortcut AUMID property failed: {err}"))?;

        let file: IPersistFile = link
            .cast()
            .map_err(|err| format!("IPersistFile cast failed: {err}"))?;
        let shortcut_w = wide_path(shortcut);
        file.Save(PCWSTR(shortcut_w.as_ptr()), true)
            .map_err(|err| format!("saving shortcut {} failed: {err}", shortcut.display()))?;
    }
    Ok(())
}

pub fn install(startup: bool) -> Result<(), String> {
    let current =
        std::env::current_exe().map_err(|err| format!("failed to locate current exe: {err}"))?;
    let installed = paths::installed_exe()?;
    if let Some(parent) = installed.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create install dir {}: {err}", parent.display()))?;
    }
    if current != installed {
        fs::copy(&current, &installed).map_err(|err| {
            format!(
                "failed to copy {} to {}: {err}",
                current.display(),
                installed.display()
            )
        })?;
    }
    crate::config::ensure_default_config(&paths::config_file()?)?;
    create_shortcut(
        &paths::start_menu_shortcut()?,
        &installed,
        None,
        paths::AUMID,
    )?;
    if startup {
        create_shortcut(&paths::startup_shortcut()?, &installed, None, paths::AUMID)?;
    }
    Ok(())
}

pub fn startup_enabled() -> bool {
    paths::startup_shortcut()
        .map(|path| path.exists())
        .unwrap_or(false)
}

pub fn set_startup_enabled(enabled: bool) -> Result<bool, String> {
    let shortcut = paths::startup_shortcut()?;
    if enabled {
        let installed = paths::installed_exe()?;
        let target = if installed.exists() {
            installed
        } else {
            std::env::current_exe().map_err(|err| format!("failed to locate current exe: {err}"))?
        };
        create_shortcut(&shortcut, &target, None, paths::AUMID)?;
        Ok(true)
    } else {
        if shortcut.exists() {
            fs::remove_file(&shortcut).map_err(|err| {
                format!(
                    "failed to remove startup shortcut {}: {err}",
                    shortcut.display()
                )
            })?;
        }
        Ok(false)
    }
}

pub fn uninstall(purge: bool) -> Result<(), String> {
    for path in [paths::start_menu_shortcut()?, paths::startup_shortcut()?] {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|err| format!("failed to remove shortcut {}: {err}", path.display()))?;
        }
    }
    if purge {
        let installed = paths::installed_exe()?;
        if installed.exists() {
            if is_current_exe(&installed)? {
                schedule_delete_after_exit(&installed)?;
            } else {
                fs::remove_file(&installed).map_err(|err| {
                    format!(
                        "failed to remove installed exe {}: {err}",
                        installed.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn is_current_exe(path: &Path) -> Result<bool, String> {
    let current =
        std::env::current_exe().map_err(|err| format!("failed to locate current exe: {err}"))?;
    let current = current.canonicalize().unwrap_or(current);
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    Ok(current == path)
}

fn schedule_delete_after_exit(path: &Path) -> Result<(), String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let helper = std::env::temp_dir().join(format!(
        "wintfy-rs-uninstall-{}-{stamp}.cmd",
        std::process::id()
    ));
    let target = batch_escape(&path.as_os_str().to_string_lossy());
    let script = format!(
        "@echo off\r\n\
         set \"target={target}\"\r\n\
         for /l %%i in (1,1,30) do (\r\n\
         \t del /f /q \"%target%\" >nul 2>nul\r\n\
         \t if not exist \"%target%\" goto done\r\n\
         \t timeout /t 1 /nobreak >nul\r\n\
         )\r\n\
         :done\r\n\
         del /f /q \"%~f0\" >nul 2>nul\r\n"
    );
    fs::write(&helper, script).map_err(|err| {
        format!(
            "failed to write uninstall helper {}: {err}",
            helper.display()
        )
    })?;
    Command::new("cmd.exe")
        .arg("/C")
        .arg(&helper)
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| {
            format!(
                "failed to start uninstall helper {}: {err}",
                helper.display()
            )
        })?;
    Ok(())
}

fn batch_escape(value: &str) -> String {
    value.replace('^', "^^").replace('%', "%%")
}

struct ComGuard;

impl ComGuard {
    fn init() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|err| format!("CoInitializeEx failed: {err}"))?;
        }
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(path: &Path) -> Vec<u16> {
    wide(&path.as_os_str().to_string_lossy())
}

unsafe fn co_task_wide(s: &str) -> Result<windows::core::PWSTR, String> {
    let wide = wide(s);
    let bytes = wide.len() * std::mem::size_of::<u16>();
    let ptr = unsafe { CoTaskMemAlloc(bytes) as *mut u16 };
    if ptr.is_null() {
        return Err("CoTaskMemAlloc failed for shortcut AUMID".to_string());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
    }
    Ok(windows::core::PWSTR(ptr))
}

fn propvariant_lpwstr(s: &str) -> Result<PROPVARIANT, String> {
    let pwsz = unsafe { co_task_wide(s)? };
    Ok(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VARENUM(VT_LPWSTR.0),
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 { pwszVal: pwsz },
            }),
        },
    })
}
