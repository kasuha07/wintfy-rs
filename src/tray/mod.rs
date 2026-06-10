use std::{
    ffi::c_void,
    mem::size_of,
    ptr::null_mut,
    sync::{Arc, Mutex, mpsc::Sender},
};

use windows::{
    Win32::{
        Foundation::{GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
                NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CREATESTRUCTW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW,
                DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
                GetCursorPos, GetMessageW, GetWindowLongPtrW, HMENU, IDC_ARROW, IDI_APPLICATION,
                LoadCursorW, LoadIconW, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG,
                PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
                SetForegroundWindow, SetWindowLongPtrW, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
                TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_COMMAND, WM_CREATE,
                WM_DESTROY, WM_POWERBROADCAST, WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPED,
            },
        },
    },
    core::PCWSTR,
};

use crate::app::AppEvent;

const WM_TRAYICON: u32 = WM_USER + 1;
const WM_QUIT_APP: u32 = WM_APP + 1;
const ID_RELOAD: usize = 1001;
const ID_OPEN_CONFIG: usize = 1002;
const ID_OPEN_LOGS: usize = 1003;
const ID_TEST_NOTIFICATION: usize = 1004;
const ID_QUIT: usize = 1005;
const ID_MUTE: usize = 1006;
const ID_STARTUP: usize = 1007;

struct TrayState {
    tx: Sender<AppEvent>,
    taskbar_created: u32,
    flags: Arc<Mutex<TrayFlags>>,
}

pub struct TrayApp {
    hwnd: HWND,
    flags: Arc<Mutex<TrayFlags>>,
}

#[derive(Debug, Default)]
struct TrayFlags {
    muted: bool,
    start_with_windows: bool,
}

struct TrayInit {
    tx: Sender<AppEvent>,
    flags: Arc<Mutex<TrayFlags>>,
}

#[derive(Clone)]
pub struct TrayHandle {
    hwnd: isize,
    flags: Arc<Mutex<TrayFlags>>,
}

unsafe impl Send for TrayHandle {}
unsafe impl Sync for TrayHandle {}

impl TrayApp {
    pub fn create(tx: Sender<AppEvent>) -> Result<Self, String> {
        unsafe {
            let class_name = wide("wintfy-rs-tray-window");
            let hinstance =
                GetModuleHandleW(None).map_err(|err| format!("GetModuleHandleW failed: {err}"))?;
            let wc = WNDCLASSW {
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hInstance: hinstance.into(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                lpfnWndProc: Some(window_proc),
                ..Default::default()
            };
            let atom = RegisterClassW(&wc);
            if atom == 0 {
                let err = GetLastError();
                if err.0 != 1410 {
                    return Err(format!("RegisterClassW failed: {err:?}"));
                }
            }
            let flags = Arc::new(Mutex::new(TrayFlags {
                muted: false,
                start_with_windows: crate::platform::shell::startup_enabled(),
            }));
            let init_ptr = Box::into_raw(Box::new(TrayInit {
                tx,
                flags: flags.clone(),
            }));
            let window_title = wide("wintfy-rs");
            let hwnd = CreateWindowExW(
                Default::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(window_title.as_ptr()),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                Some(hinstance.into()),
                Some(init_ptr.cast()),
            )
            .map_err(|err| format!("CreateWindowExW failed: {err}"))?;
            if hwnd.0 == null_mut() {
                let _ = Box::from_raw(init_ptr);
                return Err(format!(
                    "CreateWindowExW returned null: {:?}",
                    GetLastError()
                ));
            }
            add_icon(hwnd, "wintfy-rs: connecting")?;
            Ok(Self { hwnd, flags })
        }
    }

    pub fn message_loop(&self) -> i32 {
        unsafe {
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).into() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            msg.wParam.0 as i32
        }
    }

    pub fn handle(&self) -> TrayHandle {
        TrayHandle::new(self.hwnd, self.flags.clone())
    }
}

impl TrayHandle {
    fn new(hwnd: HWND, flags: Arc<Mutex<TrayFlags>>) -> Self {
        Self {
            hwnd: hwnd.0 as isize,
            flags,
        }
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut c_void)
    }

    pub fn set_tooltip(&self, tip: &str) {
        let _ = modify_icon(self.hwnd(), tip);
    }

    pub fn set_muted(&self, muted: bool) {
        if let Ok(mut flags) = self.flags.lock() {
            flags.muted = muted;
        }
    }

    pub fn set_start_with_windows(&self, enabled: bool) {
        if let Ok(mut flags) = self.flags.lock() {
            flags.start_with_windows = enabled;
        }
    }

    pub fn quit(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), WM_QUIT_APP, WPARAM(0), LPARAM(0));
        }
    }
}

impl Drop for TrayApp {
    fn drop(&mut self) {
        unsafe {
            remove_icon(self.hwnd);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CREATE {
        let createstruct = lparam.0 as *const CREATESTRUCTW;
        if !createstruct.is_null() {
            let init_ptr = unsafe { (*createstruct).lpCreateParams as *mut TrayInit };
            if !init_ptr.is_null() {
                let init = unsafe { *Box::from_raw(init_ptr) };
                let taskbar_created =
                    unsafe { RegisterWindowMessageW(PCWSTR(wide("TaskbarCreated").as_ptr())) };
                let state = Box::new(TrayState {
                    tx: init.tx,
                    taskbar_created,
                    flags: init.flags,
                });
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
                }
            }
        }
        return LRESULT(0);
    }

    let is_taskbar_created = unsafe {
        tray_state(hwnd)
            .map(|state| msg == state.taskbar_created)
            .unwrap_or(false)
    };
    if is_taskbar_created {
        let _ = unsafe { add_icon(hwnd, "wintfy-rs") };
        return LRESULT(0);
    }

    match msg {
        WM_TRAYICON => {
            if lparam.0 as u32 == WM_RBUTTONUP {
                unsafe { show_menu(hwnd) };
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xffff;
            unsafe { send_menu_event(hwnd, id) };
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if crate::platform::power::is_resume_event(wparam.0) {
                unsafe { send_event(hwnd, AppEvent::PowerResume) };
            }
            LRESULT(1)
        }
        WM_QUIT_APP => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                remove_icon(hwnd);
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayState;
                if !ptr.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(ptr));
                }
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn send_menu_event(hwnd: HWND, id: usize) {
    match id {
        ID_RELOAD => unsafe { send_event(hwnd, AppEvent::TrayReloadConfig) },
        ID_OPEN_CONFIG => unsafe { send_event(hwnd, AppEvent::TrayOpenConfig) },
        ID_OPEN_LOGS => unsafe { send_event(hwnd, AppEvent::TrayOpenLogs) },
        ID_TEST_NOTIFICATION => unsafe { send_event(hwnd, AppEvent::TrayTestNotification) },
        ID_MUTE => unsafe { send_event(hwnd, AppEvent::TrayToggleMute) },
        ID_STARTUP => unsafe { send_event(hwnd, AppEvent::TrayToggleStartWithWindows) },
        ID_QUIT => unsafe { send_event(hwnd, AppEvent::TrayQuit) },
        _ => {}
    }
}

unsafe fn send_event(hwnd: HWND, event: AppEvent) {
    if let Some(state) = unsafe { tray_state(hwnd) } {
        let _ = state.tx.send(event);
    }
}

unsafe fn tray_state(hwnd: HWND) -> Option<&'static TrayState> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const TrayState };
    unsafe { ptr.as_ref() }
}

unsafe fn show_menu(hwnd: HWND) {
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
        return;
    };
    unsafe {
        let (muted, startup) = tray_state(hwnd)
            .and_then(|state| {
                state
                    .flags
                    .lock()
                    .ok()
                    .map(|flags| (flags.muted, flags.start_with_windows))
            })
            .unwrap_or((false, false));
        append(menu, ID_OPEN_CONFIG, "Open config");
        append(menu, ID_RELOAD, "Reload config");
        append(menu, ID_OPEN_LOGS, "Logs");
        append(menu, ID_TEST_NOTIFICATION, "Test notification");
        append_checked(menu, ID_MUTE, if muted { "Unmute" } else { "Mute" }, muted);
        append_checked(menu, ID_STARTUP, "Start with Windows", startup);
        append_separator(menu);
        append(menu, ID_QUIT, "Quit");
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
    }
}

unsafe fn append(menu: HMENU, id: usize, text: &str) {
    let text = wide(text);
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(text.as_ptr()));
    }
}

unsafe fn append_checked(menu: HMENU, id: usize, text: &str, checked: bool) {
    let text = wide(text);
    let flags = if checked {
        MF_STRING | MF_CHECKED
    } else {
        MF_STRING | MF_UNCHECKED
    };
    unsafe {
        let _ = AppendMenuW(menu, flags, id, PCWSTR(text.as_ptr()));
    }
}

unsafe fn append_separator(menu: HMENU) {
    unsafe {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    }
}

unsafe fn add_icon(hwnd: HWND, tip: &str) -> Result<(), String> {
    let mut data = notify_data(hwnd, tip);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAYICON;
    data.hIcon = unsafe { LoadIconW(None, IDI_APPLICATION) }
        .map_err(|err| format!("LoadIconW failed: {err}"))?;
    if unsafe { Shell_NotifyIconW(NIM_ADD, &mut data) }.as_bool() {
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &mut data) };
        Ok(())
    } else {
        Err(format!("Shell_NotifyIconW add failed: {:?}", unsafe {
            GetLastError()
        }))
    }
}

fn modify_icon(hwnd: HWND, tip: &str) -> Result<(), String> {
    unsafe {
        let mut data = notify_data(hwnd, tip);
        data.uFlags = NIF_TIP;
        if Shell_NotifyIconW(NIM_MODIFY, &mut data).as_bool() {
            Ok(())
        } else {
            Err(format!(
                "Shell_NotifyIconW modify failed: {:?}",
                GetLastError()
            ))
        }
    }
}

unsafe fn remove_icon(hwnd: HWND) {
    let mut data = notify_data(hwnd, "wintfy-rs");
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &mut data);
    }
}

fn notify_data(hwnd: HWND, tip: &str) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    let tip_w = wide(tip);
    for (idx, value) in tip_w.into_iter().take(data.szTip.len()).enumerate() {
        data.szTip[idx] = value;
    }
    data
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
