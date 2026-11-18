use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use windows::core::{w, PCWSTR, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Custom Window Message ID nhận sự kiện từ System Tray Icon
pub const WM_TRAYICON: u32 = WM_USER + 1;

/// Message ID đăng ký với Windows Shell để nhận thông báo TaskbarCreated (khi Explorer vẽ lại taskbar)
static WM_TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Resource ID của Application Icon nhúng trong PE (khớp với app.rc: 1 ICON "...")
pub const IDI_APP_ICON: u16 = 1;

/// Nạp icon Predator (Resource ID = 1) đã nhúng trong PE executable từ HMODULE hiện tại bằng LoadImageW
/// (fallback LoadIconW(None, IDI_APPLICATION)).
pub fn load_embedded_app_icon() -> HICON {
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        // MAKEINTRESOURCEW(1) tương đương ép kiểu 1usize as *const u16
        let res_id = PCWSTR(IDI_APP_ICON as usize as *const u16);

        if let Ok(handle) = LoadImageW(
            HINSTANCE(hinstance.0),
            res_id,
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE | LR_SHARED,
        ) {
            if !handle.is_invalid() {
                return HICON(handle.0);
            }
        }

        // Fallback an toàn nếu không tìm thấy resource
        let fallback = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
        if !fallback.is_invalid() {
            return fallback;
        }

        // Fallback dự phòng cuối cùng bằng GDI thuần
        create_dynamic_fan_icon().unwrap_or_default()
    }
}

/// Menu Item Command IDs
pub const IDM_OPEN_DASHBOARD: usize = 999;
pub const IDM_STATUS: usize = 1000;
pub const IDM_PROFILE_ACTIVE_CURVE: usize = 1001;
pub const IDM_PROFILE_TURBO: usize = 1002;
pub const IDM_PROFILE_SILENT: usize = 1003;
pub const IDM_RELOAD_CONFIG: usize = 1004;
pub const IDM_EXIT: usize = 1005;
pub const IDM_CUSTOMIZE_CURVE: usize = 1006;
pub const IDM_OPEN_CONFIG_FILE: usize = 1007;
pub const IDM_TOGGLE_GOVERNOR: usize = 1008;
pub const IDM_SHOW_TEMP_CHART: usize = 1050;

/// Các Profile hoạt động của quạt Acer Predator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FanProfile {
    ActiveCurve = 0,
    Turbo = 1,
    Silent = 2,
}

impl FanProfile {
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::Turbo,
            2 => Self::Silent,
            _ => Self::ActiveCurve,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

/// Lệnh điều khiển gửi từ giao diện System Tray tới worker thread
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCommand {
    SetProfile(FanProfile),
    ReloadConfig,
    OpenCurveDialog,
    ShowTempChart,
    ToggleGovernor,
    Exit,
}

/// Trạng thái chia sẻ liên luồng (Thread-Safe Atomic State) giữa worker và UI
pub struct TrayState {
    pub cpu_temp: AtomicU32,
    pub cpu_speed: AtomicU8,
    pub gpu_temp: AtomicU32,
    pub gpu_speed: AtomicU8,
    pub system_temp: AtomicU32,
    pub cpu_rpm: AtomicU32,
    pub gpu_rpm: AtomicU32,
    pub active_profile: AtomicU8,
    pub governor_enabled: AtomicBool,
    pub governor_target_temp: AtomicU32,
    pub governor_epp: AtomicU32,
    pub governor_is_capped: AtomicBool,
    pub governor_cap_mhz: AtomicU32,
    pub cpu_power_dw: AtomicU32,
    pub gpu_power_dw: AtomicU32,
    pub cpu_mhz: AtomicU32,
    pub gpu_mhz: AtomicU32,
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            cpu_temp: AtomicU32::new(0),
            cpu_speed: AtomicU8::new(0),
            gpu_temp: AtomicU32::new(0),
            gpu_speed: AtomicU8::new(0),
            system_temp: AtomicU32::new(0),
            cpu_rpm: AtomicU32::new(0),
            gpu_rpm: AtomicU32::new(0),
            active_profile: AtomicU8::new(FanProfile::ActiveCurve.to_u8()),
            governor_enabled: AtomicBool::new(true),
            governor_target_temp: AtomicU32::new(80),
            governor_epp: AtomicU32::new(25),
            governor_is_capped: AtomicBool::new(false),
            governor_cap_mhz: AtomicU32::new(0),
            cpu_power_dw: AtomicU32::new(0),
            gpu_power_dw: AtomicU32::new(0),
            cpu_mhz: AtomicU32::new(0),
            gpu_mhz: AtomicU32::new(0),
        }
    }
}

impl TrayState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update_status(&self, cpu_temp: u32, cpu_speed: u8, gpu_temp: u32, gpu_speed: u8, cpu_rpm: u32, gpu_rpm: u32) {
        self.cpu_temp.store(cpu_temp, Ordering::Relaxed);
        self.cpu_speed.store(cpu_speed, Ordering::Relaxed);
        self.gpu_temp.store(gpu_temp, Ordering::Relaxed);
        self.gpu_speed.store(gpu_speed, Ordering::Relaxed);
        self.cpu_rpm.store(cpu_rpm, Ordering::Relaxed);
        self.gpu_rpm.store(gpu_rpm, Ordering::Relaxed);
    }

    pub fn update_metrics(&self, cpu_power_dw: u16, gpu_power_dw: u16, cpu_mhz: u16, gpu_mhz: u16) {
        self.cpu_power_dw.store(cpu_power_dw as u32, Ordering::Relaxed);
        self.gpu_power_dw.store(gpu_power_dw as u32, Ordering::Relaxed);
        self.cpu_mhz.store(cpu_mhz as u32, Ordering::Relaxed);
        self.gpu_mhz.store(gpu_mhz as u32, Ordering::Relaxed);
    }

    pub fn get_metrics(&self) -> (u16, u16, u16, u16) {
        (
            self.cpu_power_dw.load(Ordering::Relaxed) as u16,
            self.gpu_power_dw.load(Ordering::Relaxed) as u16,
            self.cpu_mhz.load(Ordering::Relaxed) as u16,
            self.gpu_mhz.load(Ordering::Relaxed) as u16,
        )
    }

    pub fn update_full_status(
        &self,
        cpu_temp: u32,
        cpu_speed: u8,
        gpu_temp: u32,
        gpu_speed: u8,
        cpu_rpm: u32,
        gpu_rpm: u32,
        cpu_power_dw: u16,
        gpu_power_dw: u16,
        cpu_mhz: u16,
        gpu_mhz: u16,
    ) {
        self.update_status(cpu_temp, cpu_speed, gpu_temp, gpu_speed, cpu_rpm, gpu_rpm);
        self.update_metrics(cpu_power_dw, gpu_power_dw, cpu_mhz, gpu_mhz);
    }

    pub fn update_governor_status(&self, enabled: bool, target_temp: u32, is_capped: bool, cap_mhz: u32) {
        self.governor_enabled.store(enabled, Ordering::Relaxed);
        self.governor_target_temp.store(target_temp, Ordering::Relaxed);
        self.governor_is_capped.store(is_capped, Ordering::Relaxed);
        self.governor_cap_mhz.store(cap_mhz, Ordering::Relaxed);
        self.governor_epp.store(if is_capped { 65 } else { 25 }, Ordering::Relaxed);
    }

    pub fn is_governor_enabled(&self) -> bool {
        self.governor_enabled.load(Ordering::Relaxed)
    }

    pub fn is_governor_capped(&self) -> bool {
        self.governor_is_capped.load(Ordering::Relaxed)
    }

    pub fn toggle_governor(&self) -> bool {
        let prev = self.governor_enabled.fetch_xor(true, Ordering::SeqCst);
        !prev
    }

    pub fn get_governor_info(&self) -> (bool, u32, bool, u32) {
        (
            self.governor_enabled.load(Ordering::Relaxed),
            self.governor_target_temp.load(Ordering::Relaxed),
            self.governor_is_capped.load(Ordering::Relaxed),
            self.governor_cap_mhz.load(Ordering::Relaxed),
        )
    }

    pub fn set_profile(&self, profile: FanProfile) {
        self.active_profile.store(profile.to_u8(), Ordering::SeqCst);
    }

    pub fn get_profile(&self) -> FanProfile {
        FanProfile::from_u8(self.active_profile.load(Ordering::SeqCst))
    }

    pub fn get_status(&self) -> (u32, u8, u32, u8) {
        (
            self.cpu_temp.load(Ordering::Relaxed),
            self.cpu_speed.load(Ordering::Relaxed),
            self.gpu_temp.load(Ordering::Relaxed),
            self.gpu_speed.load(Ordering::Relaxed),
        )
    }

    pub fn get_status_rpm(&self) -> (u32, u8, u32, u8, u32, u32) {
        (
            self.cpu_temp.load(Ordering::Relaxed),
            self.cpu_speed.load(Ordering::Relaxed),
            self.gpu_temp.load(Ordering::Relaxed),
            self.gpu_speed.load(Ordering::Relaxed),
            self.cpu_rpm.load(Ordering::Relaxed),
            self.gpu_rpm.load(Ordering::Relaxed),
        )
    }

    pub fn update_system_temp(&self, val: u32) {
        self.system_temp.store(val, Ordering::Relaxed);
    }

    pub fn get_system_temp(&self) -> u32 {
        self.system_temp.load(Ordering::Relaxed)
    }

    /// Định dạng chuỗi tooltip hiển thị chi tiết: CPU: XX°C XX.XW X.XXGHz | GPU: XX°C XX.XW X.XXGHz (| Sys: ZZ°C)
    pub fn format_tooltip(&self) -> String {
        let (cpu_temp, _cpu_speed, gpu_temp, _gpu_speed) = self.get_status();
        let (cpu_power_dw, gpu_power_dw, cpu_mhz, gpu_mhz) = self.get_metrics();
        let sys_temp = self.get_system_temp();
        if sys_temp > 0 {
            format!(
                "CPU: {}°C {:.1}W {:.2}GHz | GPU: {}°C {:.1}W {:.2}GHz | Sys: {}°C",
                cpu_temp,
                cpu_power_dw as f32 / 10.0,
                cpu_mhz as f32 / 1000.0,
                gpu_temp,
                gpu_power_dw as f32 / 10.0,
                gpu_mhz as f32 / 1000.0,
                sys_temp,
            )
        } else {
            format!(
                "CPU: {}°C {:.1}W {:.2}GHz | GPU: {}°C {:.1}W {:.2}GHz",
                cpu_temp,
                cpu_power_dw as f32 / 10.0,
                cpu_mhz as f32 / 1000.0,
                gpu_temp,
                gpu_power_dw as f32 / 10.0,
                gpu_mhz as f32 / 1000.0,
            )
        }
    }
}

/// Cấu trúc nội bộ gắn vào `GWLP_USERDATA` của cửa sổ ẩn
#[allow(dead_code)]
struct TrayContext {
    state: Arc<TrayState>,
    cmd_sender: Sender<TrayCommand>,
    hwnd: HWND,
    hicon: HICON,
    telemetry: Arc<std::sync::Mutex<crate::telemetry::TelemetryHistory>>,
}

/// Handle gửi lệnh và cập nhật dữ liệu từ bất kỳ thread nào
#[derive(Clone)]
pub struct TrayHandle {
    hwnd: HWND,
    state: Arc<TrayState>,
}

unsafe impl Send for TrayHandle {}
unsafe impl Sync for TrayHandle {}

impl TrayHandle {
    /// Cập nhật thông số nhiệt độ, tốc độ quạt (%) và RPM hiển thị tức thì trên menu
    pub fn update_status(&self, cpu_temp: u32, cpu_speed: u8, gpu_temp: u32, gpu_speed: u8, cpu_rpm: u32, gpu_rpm: u32) {
        self.state.update_status(cpu_temp, cpu_speed, gpu_temp, gpu_speed, cpu_rpm, gpu_rpm);
    }

    pub fn update_metrics(&self, cpu_power_dw: u16, gpu_power_dw: u16, cpu_mhz: u16, gpu_mhz: u16) {
        self.state.update_metrics(cpu_power_dw, gpu_power_dw, cpu_mhz, gpu_mhz);
    }

    pub fn get_metrics(&self) -> (u16, u16, u16, u16) {
        self.state.get_metrics()
    }

    pub fn update_full_status(
        &self,
        cpu_temp: u32,
        cpu_speed: u8,
        gpu_temp: u32,
        gpu_speed: u8,
        cpu_rpm: u32,
        gpu_rpm: u32,
        cpu_power_dw: u16,
        gpu_power_dw: u16,
        cpu_mhz: u16,
        gpu_mhz: u16,
    ) {
        self.state.update_full_status(
            cpu_temp, cpu_speed, gpu_temp, gpu_speed, cpu_rpm, gpu_rpm,
            cpu_power_dw, gpu_power_dw, cpu_mhz, gpu_mhz,
        );
    }

    pub fn update_governor_status(&self, enabled: bool, target_temp: u32, is_capped: bool, cap_mhz: u32) {
        self.state.update_governor_status(enabled, target_temp, is_capped, cap_mhz);
    }

    pub fn is_governor_enabled(&self) -> bool {
        self.state.is_governor_enabled()
    }

    pub fn is_governor_capped(&self) -> bool {
        self.state.is_governor_capped()
    }

    pub fn toggle_governor(&self) -> bool {
        self.state.toggle_governor()
    }

    pub fn get_governor_info(&self) -> (bool, u32, bool, u32) {
        self.state.get_governor_info()
    }

    pub fn update_system_temp(&self, val: u32) {
        self.state.update_system_temp(val);
    }

    pub fn get_system_temp(&self) -> u32 {
        self.state.get_system_temp()
    }

    /// Cập nhật tooltip khay hệ thống (hiển thị khi rê chuột vào icon)
    pub fn update_tooltip(&self, text: &str) {
        unsafe {
            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_TIP;
            copy_to_wide_buf(text, &mut nid.szTip);
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    pub fn set_profile(&self, profile: FanProfile) {
        self.state.set_profile(profile);
    }

    pub fn get_profile(&self) -> FanProfile {
        self.state.get_profile()
    }

    pub fn get_status(&self) -> (u32, u8, u32, u8) {
        self.state.get_status()
    }

    pub fn state(&self) -> Arc<TrayState> {
        self.state.clone()
    }

    /// Định dạng chuỗi tooltip trạng thái chi tiết
    pub fn format_tooltip(&self) -> String {
        self.state.format_tooltip()
    }

    /// Cập nhật tooltip theo các thông số hiện thời
    pub fn update_status_tooltip(&self) {
        let tip = self.format_tooltip();
        self.update_tooltip(&tip);
    }

    /// Yêu cầu đóng ứng dụng an toàn từ xa
    pub fn request_exit(&self) {
        unsafe {
            let _ = PostMessageW(self.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Trình quản lý System Tray Win32 Native
pub struct TrayManager {
    hwnd: HWND,
    hicon: HICON,
    state: Arc<TrayState>,
    cmd_sender: Sender<TrayCommand>,
    context_ptr: *mut TrayContext,
}

impl TrayManager {
    /// Khởi tạo System Tray Icon và cửa sổ ẩn nhận sự kiện
    ///
    /// Trả về:
    /// - `TrayManager`: đối tượng giữ vòng lặp thông điệp trên Main Thread.
    /// - `TrayHandle`: handle nhẹ có thể chuyển qua controller thread để đẩy dữ liệu.
    /// - `Receiver<TrayCommand>`: kênh nhận sự kiện khi người dùng chọn menu.
    pub fn create(
        telemetry: Arc<std::sync::Mutex<crate::telemetry::TelemetryHistory>>,
    ) -> Result<(Self, TrayHandle, Receiver<TrayCommand>)> {
        let (cmd_sender, cmd_receiver) = mpsc::channel();
        let state = Arc::new(TrayState::new());

        unsafe {
            let hinstance = GetModuleHandleW(PCWSTR::null())?;
            let class_name = w!("PredatorFanTrayWindowClass");

            // Nạp icon Predator từ PE Resource table
            let app_icon = load_embedded_app_icon();

            let wnd_class = WNDCLASSW {
                style: CS_GLOBALCLASS,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance.into(),
                hIcon: app_icon,
                hCursor: HCURSOR::default(),
                hbrBackground: HBRUSH::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: class_name,
            };

            let _ = RegisterClassW(&wnd_class);

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                w!("PredatorFanTray"),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                0,
                0,
                HWND::default(),
                HMENU::default(),
                hinstance,
                None,
            )?;

            let context = Box::new(TrayContext {
                state: state.clone(),
                cmd_sender: cmd_sender.clone(),
                hwnd,
                hicon: app_icon,
                telemetry,
            });
            let context_ptr = Box::into_raw(context);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, context_ptr as isize);

            // Cho phép các thông điệp UIPI từ tiến trình không đặc quyền (non-elevated / Explorer.exe) gửi tới
            let _ = ChangeWindowMessageFilterEx(hwnd, WM_COMMAND, MSGFLT_ALLOW, None);
            let _ = ChangeWindowMessageFilterEx(hwnd, WM_CLOSE, MSGFLT_ALLOW, None);
            let _ = ChangeWindowMessageFilterEx(hwnd, WM_TRAYICON, MSGFLT_ALLOW, None);

            // Đăng ký nhận thông báo TaskbarCreated từ Explorer khi Shell khởi động hoặc vẽ lại
            let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
            WM_TASKBAR_CREATED.store(taskbar_created, Ordering::SeqCst);
            if taskbar_created != 0 {
                let _ = ChangeWindowMessageFilterEx(hwnd, taskbar_created, MSGFLT_ALLOW, None);
            }

            // Đăng ký Icon vào Shell System Tray với cơ chế retry thông minh
            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = app_icon;
            copy_to_wide_buf("Acer Predator Fan Controller (Active Curve)", &mut nid.szTip);

            let mut added = false;
            for _ in 0..10 {
                let ok = Shell_NotifyIconW(NIM_ADD, &nid);
                if ok.as_bool() {
                    added = true;
                    break;
                }
                let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            if !added {
                eprintln!("[WARN] Shell_NotifyIconW(NIM_ADD) chưa sẵn sàng lúc boot, sẽ tự thêm lại khi nhận TaskbarCreated");
            }

            let handle = TrayHandle {
                hwnd,
                state: state.clone(),
            };

            let manager = Self {
                hwnd,
                hicon: app_icon,
                state,
                cmd_sender,
                context_ptr,
            };

            Ok((manager, handle, cmd_receiver))
        }
    }

    /// Chạy vòng lặp thông điệp Win32 Native (`GetMessageW`).
    /// - Luồng sẽ ngủ (Sleep) 100% thời gian khi không có sự kiện chuột khay hệ thống (0% CPU).
    /// - Khi người dùng chọn "Thoát" hoặc nhận `WM_QUIT`, hàm sẽ kết thúc và tự động giải phóng tài nguyên.
    pub fn run_message_loop(&self) -> Result<()> {
        unsafe {
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }

    pub fn state(&self) -> Arc<TrayState> {
        self.state.clone()
    }

    pub fn cmd_sender(&self) -> Sender<TrayCommand> {
        self.cmd_sender.clone()
    }

    #[inline]
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for TrayManager {
    fn drop(&mut self) {
        unsafe {
            // Xóa Tray Icon khỏi khay hệ thống
            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.hwnd;
            nid.uID = 1;
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);

            if !self.hicon.is_invalid() {
                let _ = DestroyIcon(self.hicon);
            }

            if !self.hwnd.is_invalid() {
                let _ = DestroyWindow(self.hwnd);
            }

            if !self.context_ptr.is_null() {
                let _ = Box::from_raw(self.context_ptr);
                self.context_ptr = std::ptr::null_mut();
            }
        }
    }
}

/// Window Procedure xử lý các sự kiện khay hệ thống
unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Nếu Explorer vừa khởi động hoặc vẽ lại Taskbar -> Tự động thêm lại icon
    let taskbar_msg = WM_TASKBAR_CREATED.load(Ordering::Relaxed);
    if taskbar_msg != 0 && msg == taskbar_msg {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayContext;
        if let Some(ctx) = ptr.as_ref() {
            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = ctx.hicon;
            copy_to_wide_buf("Acer Predator Fan Controller (Active Curve)", &mut nid.szTip);
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
        return LRESULT(0);
    }

    match msg {
        WM_TRAYICON => {
            let event = lparam.0 as u32;
            if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayContext;
                if let Some(ctx) = ptr.as_ref() {
                    show_tray_menu(hwnd, ctx.state.clone(), &ctx.cmd_sender, ctx.telemetry.clone());
                }
            } else if event == WM_LBUTTONUP {
                // Nhấp chuột trái: Mở Bảng điều khiển (Dashboard) trực quan!
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayContext;
                if let Some(ctx) = ptr.as_ref() {
                    crate::dashboard_window::show_or_focus_dashboard(
                        ctx.state.clone(),
                        ctx.cmd_sender.clone(),
                        ctx.telemetry.clone(),
                    );
                }
            } else if event == WM_LBUTTONDBLCLK {
                // Nhấp đúp chuột trái: chuyển nhanh giữa Active Curve và Full Turbo
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayContext;
                if let Some(ctx) = ptr.as_ref() {
                    let current = ctx.state.get_profile();
                    let next = if current == FanProfile::Turbo {
                        FanProfile::ActiveCurve
                    } else {
                        FanProfile::Turbo
                    };
                    ctx.state.set_profile(next);
                    let _ = ctx.cmd_sender.send(TrayCommand::SetProfile(next));
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd_id = (wparam.0 & 0xFFFF) as usize;
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayContext;
            if let Some(ctx) = ptr.as_ref() {
                match cmd_id {
                    IDM_OPEN_DASHBOARD => {
                        crate::dashboard_window::show_or_focus_dashboard(
                            ctx.state.clone(),
                            ctx.cmd_sender.clone(),
                            ctx.telemetry.clone(),
                        );
                    }
                    IDM_SHOW_TEMP_CHART => {
                        crate::chart_window::show_or_focus_chart_window(ctx.telemetry.clone());
                    }
                    IDM_CUSTOMIZE_CURVE => {
                        crate::curve_dialog::open_curve_dialog(ctx.cmd_sender.clone(), Some(ctx.state.clone()));
                    }
                    IDM_RELOAD_CONFIG => {
                        let _ = ctx.cmd_sender.send(TrayCommand::ReloadConfig);
                    }
                    IDM_TOGGLE_GOVERNOR => {
                        let _ = ctx.state.toggle_governor();
                        let _ = ctx.cmd_sender.send(TrayCommand::ToggleGovernor);
                    }
                    IDM_EXIT => {
                        let _ = ctx.cmd_sender.send(TrayCommand::Exit);
                        PostQuitMessage(0);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Hiển thị Context Menu chuột phải với thông số động và các tùy chọn Profile
unsafe fn show_tray_menu(
    hwnd: HWND,
    state: Arc<TrayState>,
    cmd_sender: &Sender<TrayCommand>,
    telemetry: Arc<std::sync::Mutex<crate::telemetry::TelemetryHistory>>,
) {
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);

    let hmenu = match CreatePopupMenu() {
        Ok(m) => m,
        Err(_) => return,
    };

    // 1. Dòng hiển thị trạng thái động (Disabled menu item làm header)
    let (cpu_temp, cpu_speed, gpu_temp, gpu_speed, cpu_rpm, gpu_rpm) = state.get_status_rpm();
    let status_str = if cpu_rpm > 0 || gpu_rpm > 0 {
        format!(
            "CPU: {}°C ({} RPM - {}%) | GPU: {}°C ({} RPM - {}%)\0",
            cpu_temp, cpu_rpm, cpu_speed, gpu_temp, gpu_rpm, gpu_speed
        )
    } else {
        format!(
            "CPU: {}°C ({}%) | GPU: {}°C ({}%)\0",
            cpu_temp, cpu_speed, gpu_temp, gpu_speed
        )
    };
    let wide_status: Vec<u16> = status_str.encode_utf16().collect();
    let _ = AppendMenuW(
        hmenu,
        MF_STRING | MF_DISABLED | MF_GRAYED,
        IDM_STATUS,
        PCWSTR(wide_status.as_ptr()),
    );

    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());

    // 1.5. Mở Bảng điều khiển (Dashboard) - In đậm mặc định theo chuẩn Win32 Tray
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_OPEN_DASHBOARD,
        w!("Mở Bảng điều khiển (Dashboard)..."),
    );
    let _ = SetMenuDefaultItem(hmenu, IDM_OPEN_DASHBOARD as u32, 0);

    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());

    // 2. Lựa chọn Profile quạt
    let current_prof = state.get_profile();
    let active_check = if current_prof == FanProfile::ActiveCurve {
        MF_CHECKED
    } else {
        MF_UNCHECKED
    };
    let turbo_check = if current_prof == FanProfile::Turbo {
        MF_CHECKED
    } else {
        MF_UNCHECKED
    };
    let silent_check = if current_prof == FanProfile::Silent {
        MF_CHECKED
    } else {
        MF_UNCHECKED
    };

    let _ = AppendMenuW(
        hmenu,
        MF_STRING | active_check,
        IDM_PROFILE_ACTIVE_CURVE,
        w!("Active Curve (0-Hysteresis)"),
    );
    let _ = AppendMenuW(
        hmenu,
        MF_STRING | turbo_check,
        IDM_PROFILE_TURBO,
        w!("Full Turbo / Max (100%)"),
    );
    let _ = AppendMenuW(
        hmenu,
        MF_STRING | silent_check,
        IDM_PROFILE_SILENT,
        w!("Silent Mode"),
    );

    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());

    // 3. Khóa nhiệt độ CPU mục tiêu (2-Phase Hysteresis Thermal Clamping)
    let (gov_enabled, gov_target, is_capped, cap_mhz) = state.get_governor_info();
    let gov_check = if gov_enabled {
        MF_CHECKED
    } else {
        MF_UNCHECKED
    };
    let gov_text = if gov_enabled {
        if is_capped {
            let cap_str = if cap_mhz > 0 {
                format!("{:.1} GHz", cap_mhz as f32 / 1000.0)
            } else {
                "3.8 GHz".to_string()
            };
            format!("Khóa nhiệt độ CPU ({}°C) [HÃM NHIỆT - {}]\0", gov_target, cap_str)
        } else {
            format!("Khóa nhiệt độ CPU ({}°C) [BẬT - Unlocked 4.90 GHz]\0", gov_target)
        }
    } else {
        format!("Khóa nhiệt độ CPU ({}°C) [TẮT]\0", gov_target)
    };
    let wide_gov: Vec<u16> = gov_text.encode_utf16().collect();
    let _ = AppendMenuW(
        hmenu,
        MF_STRING | gov_check,
        IDM_TOGGLE_GOVERNOR,
        PCWSTR(wide_gov.as_ptr()),
    );

    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());

    // 4. Biểu đồ nhiệt độ, Tự chỉnh cấu hình quạt & Mở file config
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_SHOW_TEMP_CHART,
        w!("Xem biểu đồ nhiệt độ (1 giờ)..."),
    );
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_CUSTOMIZE_CURVE,
        w!("Tự chỉnh Fan Curve (GUI)..."),
    );
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_OPEN_CONFIG_FILE,
        w!("Mở file config.toml (Notepad)"),
    );
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_RELOAD_CONFIG,
        w!("Tải lại cấu hình (Reload config.toml)"),
    );

    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());

    // 4. Thoát & Khôi phục
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_EXIT,
        w!("Thoát (Khôi phục Auto Fan & PredatorSense)"),
    );

    // Kích hoạt Menu Popup tương tác
    let cmd_id = TrackPopupMenuEx(
        hmenu,
        (TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RETURNCMD).0,
        pt.x,
        pt.y,
        hwnd,
        None,
    );

    // Win32 idiom: Đảm bảo menu đóng đúng cách khi người dùng nhấp ra ngoài
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(hmenu);

    match cmd_id.0 as usize {
        IDM_OPEN_DASHBOARD => {
            crate::dashboard_window::show_or_focus_dashboard(state, cmd_sender.clone(), telemetry);
        }
        IDM_SHOW_TEMP_CHART => {
            crate::chart_window::show_or_focus_chart_window(telemetry);
        }
        IDM_PROFILE_ACTIVE_CURVE => {
            state.set_profile(FanProfile::ActiveCurve);
            let _ = cmd_sender.send(TrayCommand::SetProfile(FanProfile::ActiveCurve));
        }
        IDM_PROFILE_TURBO => {
            state.set_profile(FanProfile::Turbo);
            let _ = cmd_sender.send(TrayCommand::SetProfile(FanProfile::Turbo));
        }
        IDM_PROFILE_SILENT => {
            state.set_profile(FanProfile::Silent);
            let _ = cmd_sender.send(TrayCommand::SetProfile(FanProfile::Silent));
        }
        IDM_TOGGLE_GOVERNOR => {
            let _ = state.toggle_governor();
            let _ = cmd_sender.send(TrayCommand::ToggleGovernor);
        }
        IDM_CUSTOMIZE_CURVE => {
            crate::curve_dialog::open_curve_dialog(cmd_sender.clone(), Some(state.clone()));
        }
        IDM_OPEN_CONFIG_FILE => {
            let config_path = crate::config::get_config_path();
            let _ = std::process::Command::new("notepad.exe").arg(config_path).spawn();
        }
        IDM_RELOAD_CONFIG => {
            let _ = cmd_sender.send(TrayCommand::ReloadConfig);
        }
        IDM_EXIT => {
            let _ = cmd_sender.send(TrayCommand::Exit);
            PostQuitMessage(0);
        }
        _ => {}
    }
}

/// Tạo biểu tượng quạt 16x16 sắc nét bằng Pure GDI (không cần nạp file .ico rời)
unsafe fn create_dynamic_fan_icon() -> Result<HICON> {
    let screen_dc = GetDC(HWND::default());
    let mem_dc = CreateCompatibleDC(screen_dc);

    // 1. Color Bitmap 16x16
    let hbm_color = CreateCompatibleBitmap(screen_dc, 16, 16);
    let old_bmp = SelectObject(mem_dc, hbm_color);

    // Bảng màu Acer Predator: Màu Cyan/Teal nổi bật (0x00FFD200) & Màu nền xám đen tối ưu (0x001A1816)
    let cyan_brush = CreateSolidBrush(COLORREF(0x00FFD200));
    let dark_brush = CreateSolidBrush(COLORREF(0x001A1816));

    // Tô nền xám đen
    let full_rect = RECT {
        left: 0,
        top: 0,
        right: 16,
        bottom: 16,
    };
    FillRect(mem_dc, &full_rect, dark_brush);

    // Vẽ vòng tròn cánh quạt Cyan ngoài
    let old_brush = SelectObject(mem_dc, cyan_brush);
    let _ = Ellipse(mem_dc, 1, 1, 15, 15);

    // Vẽ tâm trục quạt màu đen
    let _ = SelectObject(mem_dc, dark_brush);
    let _ = Ellipse(mem_dc, 5, 5, 11, 11);

    // Vẽ 4 điểm cánh quạt đối xứng tạo hình quạt quay
    let _ = SelectObject(mem_dc, cyan_brush);
    let _ = Ellipse(mem_dc, 7, 2, 9, 6);
    let _ = Ellipse(mem_dc, 7, 10, 9, 14);
    let _ = Ellipse(mem_dc, 2, 7, 6, 9);
    let _ = Ellipse(mem_dc, 10, 7, 14, 9);

    // Dọn dẹp DC
    let _ = SelectObject(mem_dc, old_brush);
    let _ = SelectObject(mem_dc, old_bmp);
    let _ = DeleteObject(cyan_brush);
    let _ = DeleteObject(dark_brush);
    let _ = DeleteDC(mem_dc);
    ReleaseDC(HWND::default(), screen_dc);

    // 2. Monochrome Mask Bitmap 16x16 (1-bit monochrome: 0 = hiển thị màu, 1 = trong suốt)
    let mut mask_bits = [0u8; 32]; // 16 dòng x 2 bytes
    for y in 0..16 {
        for x in 0..16 {
            let dx = x as f32 - 7.5;
            let dy = y as f32 - 7.5;
            // Tạo góc bo tròn biểu tượng trong suốt ngoài bán kính 7.5 px
            if dx * dx + dy * dy > 7.5 * 7.5 {
                let byte_idx = y * 2 + (x / 8);
                let bit_idx = 7 - (x % 8);
                mask_bits[byte_idx] |= 1 << bit_idx;
            }
        }
    }

    let hbm_mask = CreateBitmap(16, 16, 1, 1, Some(mask_bits.as_ptr() as *const _));

    let icon_info = ICONINFO {
        fIcon: BOOL(1),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: hbm_mask,
        hbmColor: hbm_color,
    };

    let hicon = CreateIconIndirect(&icon_info)?;
    let _ = DeleteObject(hbm_mask);
    let _ = DeleteObject(hbm_color);

    Ok(hicon)
}

/// Helper sao chép chuỗi UTF-8 sang bộ đệm UTF-16 có độ dài cố định
fn copy_to_wide_buf(src: &str, dest: &mut [u16]) {
    let mut idx = 0;
    for code_unit in src.encode_utf16() {
        if idx >= dest.len() - 1 {
            break;
        }
        dest[idx] = code_unit;
        idx += 1;
    }
    dest[idx] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tray_state_atomic_updates() {
        let state = TrayState::new();
        assert_eq!(state.get_status(), (0, 0, 0, 0));
        assert_eq!(state.get_status_rpm(), (0, 0, 0, 0, 0, 0));
        assert_eq!(state.get_profile(), FanProfile::ActiveCurve);

        state.update_status(55, 45, 62, 50, 2400, 2100);
        assert_eq!(state.get_status(), (55, 45, 62, 50));
        assert_eq!(state.get_status_rpm(), (55, 45, 62, 50, 2400, 2100));

        state.set_profile(FanProfile::Turbo);
        assert_eq!(state.get_profile(), FanProfile::Turbo);

        state.set_profile(FanProfile::Silent);
        assert_eq!(state.get_profile(), FanProfile::Silent);

        // Kiểm tra Thermal Governor State
        assert!(state.is_governor_enabled());
        assert!(!state.is_governor_capped());
        assert_eq!(state.get_governor_info(), (true, 80, false, 0));

        state.update_governor_status(true, 85, true, 3800);
        assert_eq!(state.get_governor_info(), (true, 85, true, 3800));
        assert!(state.is_governor_capped());

        let toggled = state.toggle_governor();
        assert!(!toggled);
        assert!(!state.is_governor_enabled());
        assert_eq!(state.get_governor_info(), (false, 85, true, 3800));

        // Kiểm tra Metrics (Power & Clock)
        assert_eq!(state.get_metrics(), (0, 0, 0, 0));
        state.update_metrics(428, 195, 3820, 2250);
        assert_eq!(state.get_metrics(), (428, 195, 3820, 2250));

        // Kiểm tra update_full_status
        state.update_full_status(60, 50, 65, 55, 2600, 2300, 450, 210, 4100, 2300);
        assert_eq!(state.get_status_rpm(), (60, 50, 65, 55, 2600, 2300));
        assert_eq!(state.get_metrics(), (450, 210, 4100, 2300));

        // Kiểm tra format_tooltip: "CPU: XX°C XX.XW X.XXGHz | GPU: XX°C XX.XW X.XXGHz"
        let tooltip = state.format_tooltip();
        assert_eq!(tooltip, "CPU: 60°C 45.0W 4.10GHz | GPU: 65°C 21.0W 2.30GHz");

        // Kiểm tra System Temp và format_tooltip có Sys
        assert_eq!(state.get_system_temp(), 0);
        state.update_system_temp(52);
        assert_eq!(state.get_system_temp(), 52);
        let tooltip_with_sys = state.format_tooltip();
        assert_eq!(tooltip_with_sys, "CPU: 60°C 45.0W 4.10GHz | GPU: 65°C 21.0W 2.30GHz | Sys: 52°C");
    }

    #[test]
    fn test_profile_serialization() {
        assert_eq!(FanProfile::from_u8(0), FanProfile::ActiveCurve);
        assert_eq!(FanProfile::from_u8(1), FanProfile::Turbo);
        assert_eq!(FanProfile::from_u8(2), FanProfile::Silent);
        assert_eq!(FanProfile::from_u8(99), FanProfile::ActiveCurve);
    }

    #[test]
    fn test_copy_to_wide_buf() {
        let mut buf = [0u16; 8];
        copy_to_wide_buf("Predator", &mut buf);
        // Chữ "Predator" có 8 ký tự, bộ đệm 8 ô thì chỉ chứa được 7 ký tự + null terminator
        assert_eq!(buf[7], 0);
        let s = String::from_utf16_lossy(&buf[..7]);
        assert_eq!(s, "Predato");
    }

    #[test]
    fn test_load_embedded_app_icon() {
        let icon = load_embedded_app_icon();
        assert!(!icon.is_invalid());
    }

    #[test]
    fn test_tray_window_creation_and_find() {
        let telemetry = Arc::new(std::sync::Mutex::new(crate::telemetry::TelemetryHistory::new()));
        let (tray, _handle, _rx) = TrayManager::create(telemetry).expect("TrayManager::create failed");
        let hwnd = tray.hwnd();
        assert!(!hwnd.0.is_null());
        unsafe {
            let found = FindWindowW(w!("PredatorFanTrayWindowClass"), w!("PredatorFanTray"));
            assert!(found.is_ok(), "FindWindowW must find PredatorFanTray");
            assert_eq!(found.unwrap(), hwnd);
        }
    }
}
