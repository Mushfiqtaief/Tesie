use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW,
    PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetForegroundWindow,
    TrackPopupMenu, TranslateMessage, CW_USEDEFAULT, HICON, ICONINFO, MF_CHECKED, MF_SEPARATOR,
    MF_STRING, MF_UNCHECKED, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_CLOSE,
    WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP, WM_USER, WNDCLASSEXW,
};

pub const TRAY_WINDOW_CLASS: &str = "TesieTrayWindowClass";
const WM_TRAY_CALLBACK: u32 = WM_USER + 42;
const TRAY_ICON_ID: u32 = 1;

const ID_OPEN_DASHBOARD: usize = 1001;
const ID_TOGGLE_AUTOSTART: usize = 1002;
const ID_OPEN_DATA_DIR: usize = 1003;
const ID_RESTART_GATEWAY: usize = 1004;
const ID_QUIT_TESIE: usize = 1005;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct DaemonControl {
    pub quit: AtomicBool,
    pub restart: AtomicBool,
    pub child_pid: AtomicU32,
    pub port: u16,
    pub data_dir: PathBuf,
    pub daemon_exe: PathBuf,
}

impl DaemonControl {
    pub fn new(port: u16, data_dir: PathBuf, daemon_exe: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            quit: AtomicBool::new(false),
            restart: AtomicBool::new(false),
            child_pid: AtomicU32::new(0),
            port,
            data_dir,
            daemon_exe,
        })
    }

    pub fn kill_active_child(&self) {
        use std::os::windows::process::CommandExt;
        let pid = self.child_pid.load(Ordering::Acquire);
        if pid != 0 {
            let pid_str = pid.to_string();
            let _ = Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid_str])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
        #[cfg(not(test))]
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "tesie-core.exe"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }

    pub fn request_restart(&self) {
        self.restart.store(true, Ordering::Release);
        self.kill_active_child();
    }

    pub fn request_quit(&self) {
        self.quit.store(true, Ordering::Release);
        let _ = std::fs::write(self.data_dir.join("daemon.stop"), b"stop");
        self.kill_active_child();
    }
}

struct TrayAppState {
    control: Arc<DaemonControl>,
    hicon: HICON,
    taskbar_created_msg: u32,
}

static TRAY_STATE: OnceLock<TrayAppState> = OnceLock::new();

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Sends `WM_CLOSE` to an already-running tray window so it deletes its taskbar icon before exit.
pub fn post_close_to_running_tray() -> bool {
    let class_w = to_wide(TRAY_WINDOW_CLASS);
    unsafe {
        let hwnd = FindWindowW(class_w.as_ptr(), std::ptr::null());
        if hwnd != 0 {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            true
        } else {
            false
        }
    }
}

pub fn run_tray_loop(control: Arc<DaemonControl>) {
    let hicon = create_tesie_tray_icon();
    let taskbar_msg_w = to_wide("TaskbarCreated");
    let taskbar_created_msg = unsafe { RegisterWindowMessageW(taskbar_msg_w.as_ptr()) };

    let _ = TRAY_STATE.set(TrayAppState {
        control: Arc::clone(&control),
        hicon,
        taskbar_created_msg,
    });

    unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let class_w = to_wide(TRAY_WINDOW_CLASS);
        let title_w = to_wide("Tesie Background Tray");

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(tray_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: hicon,
            hCursor: 0,
            hbrBackground: 0,
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_w.as_ptr(),
            hIconSm: hicon,
        };
        RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            0,
            class_w.as_ptr(),
            title_w.as_ptr(),
            0,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            0,
            0,
            hinstance,
            std::ptr::null(),
        );
        if hwnd == 0 {
            // If running in headless / non-interactive session or window creation failed,
            // keep the daemon supervisor alive in a wait loop instead of terminating
            while !control.quit.load(Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            return;
        }

        add_notify_icon(hwnd, hicon, control.port);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        remove_notify_icon(hwnd);
        if hicon != 0 {
            DestroyIcon(hicon);
        }
    }
}

unsafe fn add_notify_icon(hwnd: HWND, hicon: HICON, port: u16) {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ICON_ID;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY_CALLBACK;
    nid.hIcon = hicon;

    let tip = format!("Tesie — AI Routing Gateway (127.0.0.1:{port})");
    let tip_w = to_wide(&tip);
    let copy_len = tip_w.len().min(nid.szTip.len().saturating_sub(1));
    nid.szTip[..copy_len].copy_from_slice(&tip_w[..copy_len]);
    unsafe {
        Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

unsafe fn remove_notify_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ICON_ID;
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn open_dashboard(port: u16) {
    use std::os::windows::process::CommandExt;
    let url = format!("http://127.0.0.1:{port}");
    let _ = Command::new("cmd")
        .args(["/C", "start", "", &url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

fn open_data_folder(path: &PathBuf) {
    use std::os::windows::process::CommandExt;
    let _ = Command::new("explorer.exe")
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(st) = TRAY_STATE.get() {
        if st.taskbar_created_msg != 0 && msg == st.taskbar_created_msg {
            unsafe {
                add_notify_icon(hwnd, st.hicon, st.control.port);
            }
            return 0;
        }
    }

    match msg {
        WM_TRAY_CALLBACK => {
            let event = (lparam as u32) & 0xFFFF;
            match event {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    if let Some(st) = TRAY_STATE.get() {
                        open_dashboard(st.control.port);
                    }
                }
                WM_RBUTTONUP => unsafe {
                    show_context_menu(hwnd);
                },
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            let cmd_id = wparam & 0xFFFF;
            if let Some(st) = TRAY_STATE.get() {
                match cmd_id {
                    ID_OPEN_DASHBOARD => open_dashboard(st.control.port),
                    ID_TOGGLE_AUTOSTART => {
                        let cur = tesie_admin::system::is_autostart_enabled();
                        let _ = tesie_admin::system::set_autostart_enabled(
                            !cur,
                            &st.control.daemon_exe,
                        );
                    }
                    ID_OPEN_DATA_DIR => open_data_folder(&st.control.data_dir),
                    ID_RESTART_GATEWAY => st.control.request_restart(),
                    ID_QUIT_TESIE => {
                        st.control.request_quit();
                        unsafe {
                            remove_notify_icon(hwnd);
                            PostQuitMessage(0);
                        }
                    }
                    _ => {}
                }
            }
            0
        }
        WM_CLOSE => {
            if let Some(st) = TRAY_STATE.get() {
                st.control.request_quit();
            }
            unsafe {
                remove_notify_icon(hwnd);
                PostQuitMessage(0);
            }
            0
        }
        WM_DESTROY => {
            if let Some(st) = TRAY_STATE.get() {
                st.control.request_quit();
            }
            unsafe {
                remove_notify_icon(hwnd);
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn show_context_menu(hwnd: HWND) {
    let menu = unsafe { CreatePopupMenu() };
    if menu == 0 {
        return;
    }

    let autostart = tesie_admin::system::is_autostart_enabled();
    let autostart_flags = MF_STRING | if autostart { MF_CHECKED } else { MF_UNCHECKED };

    let lbl_open = to_wide("Open Tesie Dashboard");
    let lbl_boot = to_wide("Start on Windows Boot");
    let lbl_folder = to_wide("Open Data Folder");
    let lbl_restart = to_wide("Restart Gateway");
    let lbl_quit = to_wide("Quit Tesie");

    unsafe {
        AppendMenuW(menu, MF_STRING, ID_OPEN_DASHBOARD, lbl_open.as_ptr());
        AppendMenuW(menu, autostart_flags, ID_TOGGLE_AUTOSTART, lbl_boot.as_ptr());
        AppendMenuW(menu, MF_STRING, ID_OPEN_DATA_DIR, lbl_folder.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, ID_RESTART_GATEWAY, lbl_restart.as_ptr());
        AppendMenuW(menu, MF_STRING, ID_QUIT_TESIE, lbl_quit.as_ptr());

        let mut pt: POINT = std::mem::zeroed();
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        TrackPopupMenu(
            menu,
            TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
    }
}

fn sd_rounded_box(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - hw + r;
    let qy = (py - cy).abs() - hh + r;
    let out_len = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let in_len = qx.max(qy).min(0.0);
    out_len + in_len - r
}

fn sd_circle(px: f32, py: f32, cx: f32, cy: f32, r: f32) -> f32 {
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - r
}

/// Builds a crisp 32x32 Ocean Blue + White ARGB `HICON` matching Tesie's minimal SVG logo.
fn create_tesie_tray_icon() -> HICON {
    const SIZE: usize = 32;
    let mut bi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bi.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: SIZE as i32,
        biHeight: -(SIZE as i32), // top-down DIB
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: (SIZE * SIZE * 4) as u32,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };

    let mut bits_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let hbm_color: HBITMAP = unsafe {
        CreateDIBSection(0, &bi, DIB_RGB_COLORS, &mut bits_ptr, 0, 0)
    };
    if hbm_color == 0 || bits_ptr.is_null() {
        return 0;
    }

    let pixels = unsafe { std::slice::from_raw_parts_mut(bits_ptr as *mut u32, SIZE * SIZE) };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            // Background rounded square in Ocean Blue (#0EA5E9 top-left -> #0284C7 bottom-right)
            let d_bg = sd_rounded_box(px, py, 16.0, 16.0, 14.5, 14.5, 7.0);
            let bg_alpha = (0.65 - d_bg).clamp(0.0, 1.0);
            if bg_alpha <= 0.001 {
                pixels[y * SIZE + x] = 0;
                continue;
            }

            let t = ((px + py) / 64.0).clamp(0.0, 1.0);
            let mut r = 14.0 + t * (2.0 - 14.0);
            let mut g = 165.0 + t * (132.0 - 165.0);
            let mut b = 233.0 + t * (199.0 - 233.0);

            // Subtle upper-rim Ocean Sky highlight
            if d_bg > -1.3 && d_bg <= 0.25 && py < 16.0 {
                let rim = ((16.0 - py) / 16.0).clamp(0.0, 1.0) * 0.28;
                r = r + (255.0 - r) * rim;
                g = g + (255.0 - g) * rim;
                b = b + (255.0 - b) * rim;
            }

            // Crisp White minimal T-gateway glyph + 2 side routing nodes
            let d_top = sd_rounded_box(px, py, 16.0, 10.5, 8.2, 2.0, 2.0);
            let d_stem = sd_rounded_box(px, py, 16.0, 17.5, 2.0, 7.0, 2.0);
            let d_left_node = sd_circle(px, py, 9.6, 19.6, 2.15);
            let d_right_node = sd_circle(px, py, 22.4, 19.6, 2.15);

            let d_glyph = d_top.min(d_stem).min(d_left_node).min(d_right_node);
            let fg_alpha = (0.65 - d_glyph).clamp(0.0, 1.0);

            r = r + (255.0 - r) * fg_alpha;
            g = g + (255.0 - g) * fg_alpha;
            b = b + (255.0 - b) * fg_alpha;

            let a = (bg_alpha * 255.0).round() as u32;
            let pr = ((r * bg_alpha).round() as u32).min(255);
            let pg = ((g * bg_alpha).round() as u32).min(255);
            let pb = ((b * bg_alpha).round() as u32).min(255);
            pixels[y * SIZE + x] = (a << 24) | (pr << 16) | (pg << 8) | pb;
        }
    }

    let hbm_mask: HBITMAP = unsafe { CreateBitmap(SIZE as i32, SIZE as i32, 1, 1, std::ptr::null()) };
    let info = ICONINFO {
        fIcon: 1,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: hbm_mask,
        hbmColor: hbm_color,
    };
    let hicon = unsafe { CreateIconIndirect(&info) };

    unsafe {
        if hbm_color != 0 {
            DeleteObject(hbm_color);
        }
        if hbm_mask != 0 {
            DeleteObject(hbm_mask);
        }
    }
    hicon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_tray_icon_and_window_and_closes_on_wm_close() {
        let icon = create_tesie_tray_icon();
        assert_ne!(icon, 0, "expected non-zero 32x32 tray HICON");
        unsafe {
            DestroyIcon(icon);
        }

        let tmp_dir = std::env::temp_dir().join(format!(
            "tesie-tray-unit-test-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp_dir);
        let control = DaemonControl::new(
            20129,
            tmp_dir.clone(),
            PathBuf::from("tesie-daemon.exe"),
        );

        let tray_thread = std::thread::spawn(move || {
            run_tray_loop(control);
        });

        let class_w = to_wide(TRAY_WINDOW_CLASS);
        let mut found_hwnd: HWND = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            found_hwnd = unsafe { FindWindowW(class_w.as_ptr(), std::ptr::null()) };
            if found_hwnd != 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert_ne!(found_hwnd, 0, "expected TesieTrayWindowClass HWND on current desktop");
        assert!(post_close_to_running_tray(), "expected WM_CLOSE delivery to tray window");
        tray_thread.join().expect("tray loop should exit cleanly after WM_CLOSE");

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
