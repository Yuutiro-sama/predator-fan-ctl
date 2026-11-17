//! Module Giao diện Cấu hình Fan Curve Win32 Thuần (Pure Win32 Curve Editor Dialog)
//!
//! Cung cấp cửa sổ cấu hình trực quan, siêu nhẹ (RAM < 2MB, CPU 0% khi không thao tác):
//! - Cho phép người dùng trực tiếp sửa mốc nhiệt độ (°C) và tốc độ quạt (%) cho CPU & GPU.
//! - Các nút preset cấu hình nhanh: Mặc định (Default), Tản nhiệt cao (Gaming), Yên tĩnh (Silent).
//! - Tự động kiểm tra tính hợp lệ (Validation & Sanitization), tự động sắp xếp mốc nhiệt độ.
//! - Lưu trực tiếp vào `config.toml` và gửi lệnh Hot-Reload tức thì tới Controller Thread.
//! - Đảm bảo tính duy nhất (Singleton): nếu cửa sổ đang mở thì tự động kích hoạt lên phía trước.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::thread;

use windows::core::{w, PCWSTR, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, GetDC, GetDeviceCaps, GetSysColor,
    GetSysColorBrush, InvalidateRect, ReleaseDC, SetBkColor, SetBkMode, SetTextColor,
    UpdateWindow, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_BTNFACE, DEFAULT_CHARSET,
    FW_SEMIBOLD, HDC, HFONT, LOGPIXELSX, OPAQUE, OUT_DEFAULT_PRECIS, VARIABLE_PITCH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::Config;
use crate::curve::CurvePoint;
use crate::tray::TrayCommand;

/// HWND của cửa sổ cấu hình đang mở (Singleton)
static ACTIVE_DIALOG_HWND: AtomicIsize = AtomicIsize::new(0);

// Control IDs
const ID_CPU_TEMP_BASE: usize = 100;
const ID_CPU_SPEED_BASE: usize = 200;
const ID_GPU_TEMP_BASE: usize = 110;
const ID_GPU_SPEED_BASE: usize = 210;

const ID_BTN_PRESET_DEFAULT: usize = 301;
const ID_BTN_PRESET_GAMING: usize = 302;
const ID_BTN_PRESET_SILENT: usize = 303;

const ID_BTN_SAVE_APPLY: usize = 401;
const ID_BTN_OPEN_CONFIG: usize = 402;
const ID_BTN_CLOSE: usize = 403;

const ID_CHK_GOVERNOR_ENABLED: usize = 501;
const ID_EDIT_GOVERNOR_TARGET: usize = 502;

const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: usize = 1;
const BST_UNCHECKED: usize = 0;

use std::sync::Arc;

/// Helper tỉ lệ tọa độ theo DPI (MulDiv(val, dpi, 96))
#[inline]
pub fn scale_dpi(val: i32, dpi: u32) -> i32 {
    (val * dpi as i32 + 48) / 96
}

/// Tính toán chiều cao Font theo Point Size chuẩn Em-Height (-MulDiv(point_size, dpi, 72))
#[inline]
pub fn font_height_for_pt(pt: f32, dpi: u32) -> i32 {
    let h = (pt * (dpi as f32) / 72.0).round() as i32;
    -h.max(1)
}

/// Truy vấn DPI của cửa sổ Win32 (GetDpiForWindow, fallback 96 nếu không khả dụng)
pub unsafe fn get_window_dpi(hwnd: HWND) -> u32 {
    type GetDpiForWindowFn = unsafe extern "system" fn(HWND) -> u32;
    if let Ok(user32) = windows::Win32::System::LibraryLoader::GetModuleHandleW(w!("user32.dll")) {
        if let Some(func) = windows::Win32::System::LibraryLoader::GetProcAddress(
            user32,
            windows::core::s!("GetDpiForWindow"),
        ) {
            let func: GetDpiForWindowFn = std::mem::transmute(func);
            let dpi = func(hwnd);
            if dpi > 0 {
                return dpi;
            }
        }
    }
    96
}

/// Helper tạo HFONT Segoe UI DPI-Aware chuẩn Em-Height (-MulDiv(point_size, dpi, 72))
unsafe fn create_dialog_dpi_font(dpi: u32, point_size: f32, weight: i32) -> HFONT {
    let height = font_height_for_pt(point_size, dpi);
    CreateFontW(
        height,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        VARIABLE_PITCH.0 as u32,
        w!("Segoe UI"),
    )
}

/// Cấu trúc lưu trữ handles của các Edit control để đọc/ghi dữ liệu
struct DialogControls {
    cpu_temp_edits: [HWND; 4],
    cpu_speed_edits: [HWND; 4],
    gpu_temp_edits: [HWND; 3],
    gpu_speed_edits: [HWND; 3],
    gov_enabled_chk: HWND,
    gov_target_edit: HWND,
    telemetry_lbl: HWND,
    last_telemetry: String,
    state: Option<Arc<crate::tray::TrayState>>,
    cmd_sender: Sender<TrayCommand>,
    font_main: HFONT,
    font_btn: HFONT,
}

/// Mở cửa sổ cấu hình Fan Curve (Chạy trên một thread độc lập để không khóa System Tray)
pub fn open_curve_dialog(cmd_sender: Sender<TrayCommand>, state: Option<Arc<crate::tray::TrayState>>) {
    // Nếu cửa sổ đã mở, kích hoạt lên foreground
    let existing = ACTIVE_DIALOG_HWND.load(Ordering::Relaxed);
    if existing != 0 {
        let hwnd = HWND(existing as *mut _);
        unsafe {
            if IsWindow(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
                let _ = BringWindowToTop(hwnd);
                return;
            }
        }
    }

    thread::spawn(move || {
        if let Err(e) = run_dialog_window(cmd_sender, state) {
            eprintln!("[CurveDialog] Lỗi khởi tạo cửa sổ cấu hình: {:?}", e);
        }
    });
}

/// Mở trực tiếp cửa sổ Curve Dialog ở chế độ độc lập (Standalone Preview / Testing)
pub fn show_dialog_standalone() -> Result<()> {
    let (tx, _rx) = std::sync::mpsc::channel();
    match run_dialog_window(tx, None) {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("[ERROR] run_dialog_window failed: {:?}", e);
            Err(e)
        }
    }
}

/// Hàm chạy vòng lặp thông điệp của cửa sổ cấu hình
fn run_dialog_window(cmd_sender: Sender<TrayCommand>, state: Option<Arc<crate::tray::TrayState>>) -> Result<()> {
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null())?;
        let class_name = w!("PredatorFanCurveEditorClass");

        let bg_brush = CreateSolidBrush(COLORREF(0x00F0F0F0)); // Màu nền xám sáng tiêu chuẩn Windows

        let app_icon = crate::tray::load_embedded_app_icon();

        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(dialog_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance.into(),
            hIcon: app_icon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: bg_brush,
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
        };

        let _ = RegisterClassW(&wnd_class);

        // Tính toán kích thước cửa sổ và căn giữa màn hình thích ứng DPI
        let hdc_screen = GetDC(HWND::default());
        let initial_dpi = GetDeviceCaps(hdc_screen, LOGPIXELSX) as u32;
        let _ = ReleaseDC(HWND::default(), hdc_screen);
        let initial_dpi = if initial_dpi > 0 { initial_dpi } else { 96 };

        let client_w = scale_dpi(740, initial_dpi);
        let client_h = scale_dpi(550, initial_dpi);
        let window_style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE;
        let ex_style = WS_EX_APPWINDOW | WS_EX_TOPMOST;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: client_w,
            bottom: client_h,
        };
        let _ = AdjustWindowRectEx(&mut rect, window_style, false, ex_style);
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let pos_x = (screen_w - width) / 2;
        let pos_y = (screen_h - height) / 2;

        let hwnd = CreateWindowExW(
            ex_style,
            class_name,
            w!("Acer Predator - Cấu hình Fan Curve (Zero-Hysteresis)"),
            window_style,
            pos_x,
            pos_y,
            width,
            height,
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        )?;

        // Thiết lập biểu tượng cho title bar và Alt+Tab
        if !app_icon.is_invalid() {
            let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_BIG as _), LPARAM(app_icon.0 as _));
            let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_SMALL as _), LPARAM(app_icon.0 as _));
        }

        ACTIVE_DIALOG_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

        // Khởi tạo các Win32 UI controls
        let controls = create_dialog_controls(hwnd, cmd_sender, state)?;
        let controls_ptr = Box::into_raw(Box::new(controls));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, controls_ptr as isize);

        // Nạp cấu hình hiện tại từ config.toml vào các ô nhập
        load_config_into_controls(controls_ptr);

        // Đăng ký timer 500ms để cập nhật thông số telemetry liên tục
        let _ = SetTimer(hwnd, 101, 500, None);

        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = UpdateWindow(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = BringWindowToTop(hwnd);

        // Vòng lặp thông điệp
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        ACTIVE_DIALOG_HWND.store(0, Ordering::Relaxed);
        let _ = DeleteObject(bg_brush);
        Ok(())
    }
}

/// Tạo tất cả các Label, GroupBox, Edit và Button controls bằng Pure Win32 (DPI-Aware Segoe UI)
unsafe fn create_dialog_controls(
    parent: HWND,
    cmd_sender: Sender<TrayCommand>,
    state: Option<Arc<crate::tray::TrayState>>,
) -> Result<DialogControls> {
    let hinstance = GetModuleHandleW(PCWSTR::null())?;
    let dpi = get_window_dpi(parent);
    let s = |v: i32| scale_dpi(v, dpi);

    // Font nhãn và ô nhập: 12 pt Semi-Bold Segoe UI DPI-aware
    let font_main = create_dialog_dpi_font(dpi, 12.0, FW_SEMIBOLD.0 as i32);
    // Font nút bấm và groupbox header: 12.5 pt Semi-Bold Segoe UI DPI-aware
    let font_btn = create_dialog_dpi_font(dpi, 12.5, FW_SEMIBOLD.0 as i32);

    // Tiêu đề hướng dẫn
    let h_info = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("STATIC"),
        w!("Thiết lập nhiệt độ (°C) và tốc độ quạt (%). Phản ứng tức thì (Zero-Hysteresis)."),
        WS_CHILD | WS_VISIBLE,
        s(20), s(12), s(700), s(24),
        parent, HMENU::default(), hinstance, None,
    )?;
    SendMessageW(h_info, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

    // --- GROUP BOX CPU ---
    let h_cpu_group = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!(" Đường cong quạt CPU (CPU Fan Curve) "),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0007), // BS_GROUPBOX = 0x0007
        s(20), s(38), s(700), s(146),
        parent, HMENU::default(), hinstance, None,
    )?;
    SendMessageW(h_cpu_group, WM_SETFONT, WPARAM(font_btn.0 as usize), LPARAM(1));

    let mut cpu_temp_edits = [HWND::default(); 4];
    let mut cpu_speed_edits = [HWND::default(); 4];

    for i in 0..4 {
        let y = s(64 + (i as i32 * 27));
        let label_text = format!("Mốc {}:\0", i + 1);
        let wide_label: Vec<u16> = label_text.encode_utf16().collect();
        let h_lbl = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            PCWSTR(wide_label.as_ptr()),
            WS_CHILD | WS_VISIBLE,
            s(36), y + s(2), s(60), s(22),
            parent, HMENU::default(), hinstance, None,
        )?;
        SendMessageW(h_lbl, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

        // Nhãn Nhiệt độ
        let h_t_lbl = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Nhiệt độ:"),
            WS_CHILD | WS_VISIBLE,
            s(102), y + s(2), s(85), s(22),
            parent, HMENU::default(), hinstance, None,
        )?;
        SendMessageW(h_t_lbl, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

        // Edit box Nhiệt độ (ES_NUMBER = 0x2000 | WS_BORDER = 0x00800000)
        let h_t_edit = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!("0"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(0x2000 | 0x0001), // ES_NUMBER | ES_CENTER
            s(192), y, s(52), s(24),
            parent, HMENU((ID_CPU_TEMP_BASE + i) as *mut _), hinstance, None,
        )?;
        SendMessageW(h_t_edit, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));
        cpu_temp_edits[i] = h_t_edit;

        let h_c_unit = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("°C   -->   Tốc độ quạt:"),
            WS_CHILD | WS_VISIBLE,
            s(256), y + s(2), s(200), s(22),
            parent, HMENU::default(), hinstance, None,
        )?;
        SendMessageW(h_c_unit, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

        // Edit box Tốc độ
        let h_s_edit = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!("0"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(0x2000 | 0x0001),
            s(466), y, s(52), s(24),
            parent, HMENU((ID_CPU_SPEED_BASE + i) as *mut _), hinstance, None,
        )?;
        SendMessageW(h_s_edit, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));
        cpu_speed_edits[i] = h_s_edit;

        let h_pct = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("%"),
            WS_CHILD | WS_VISIBLE,
            s(526), y + s(2), s(30), s(22),
            parent, HMENU::default(), hinstance, None,
        )?;
        SendMessageW(h_pct, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));
    }

    // --- GROUP BOX GPU ---
    let h_gpu_group = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!(" Đường cong quạt GPU (GPU Fan Curve) "),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0007),
        s(20), s(192), s(700), s(122),
        parent, HMENU::default(), hinstance, None,
    )?;
    SendMessageW(h_gpu_group, WM_SETFONT, WPARAM(font_btn.0 as usize), LPARAM(1));

    let mut gpu_temp_edits = [HWND::default(); 3];
    let mut gpu_speed_edits = [HWND::default(); 3];

    for i in 0..3 {
        let y = s(218 + (i as i32 * 27));
        let label_text = format!("Mốc {}:\0", i + 1);
        let wide_label: Vec<u16> = label_text.encode_utf16().collect();
        let h_lbl = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            PCWSTR(wide_label.as_ptr()),
            WS_CHILD | WS_VISIBLE,
            s(36), y + s(2), s(60), s(22),
            parent, HMENU::default(), hinstance, None,
        )?;
        SendMessageW(h_lbl, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

        let h_t_lbl = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Nhiệt độ:"),
            WS_CHILD | WS_VISIBLE,
            s(102), y + s(2), s(85), s(22),
            parent, HMENU::default(), hinstance, None,
        )?;
        SendMessageW(h_t_lbl, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

        let h_t_edit = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!("0"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(0x2000 | 0x0001),
            s(192), y, s(52), s(24),
            parent, HMENU((ID_GPU_TEMP_BASE + i) as *mut _), hinstance, None,
        )?;
        SendMessageW(h_t_edit, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));
        gpu_temp_edits[i] = h_t_edit;

        let h_c_unit = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("°C   -->   Tốc độ quạt:"),
            WS_CHILD | WS_VISIBLE,
            s(256), y + s(2), s(200), s(22),
            parent, HMENU::default(), hinstance, None,
        )?;
        SendMessageW(h_c_unit, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

        let h_s_edit = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("EDIT"),
            w!("0"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(0x2000 | 0x0001),
            s(466), y, s(52), s(24),
            parent, HMENU((ID_GPU_SPEED_BASE + i) as *mut _), hinstance, None,
        )?;
        SendMessageW(h_s_edit, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));
        gpu_speed_edits[i] = h_s_edit;

        let h_pct = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("%"),
            WS_CHILD | WS_VISIBLE,
            s(526), y + s(2), s(30), s(22),
            parent, HMENU::default(), hinstance, None,
        )?;
        SendMessageW(h_pct, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));
    }

    // --- GROUP BOX THERMAL GOVERNOR ---
    let h_gov_group = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!(" Khóa nhiệt độ CPU mục tiêu (Smooth EPP Governor) "),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0007), // BS_GROUPBOX
        s(20), s(322), s(700), s(84),
        parent, HMENU::default(), hinstance, None,
    )?;
    SendMessageW(h_gov_group, WM_SETFONT, WPARAM(font_btn.0 as usize), LPARAM(1));

    // Checkbox: Kích hoạt Khóa nhiệt độ CPU
    let h_gov_chk = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Kích hoạt Khóa nhiệt độ CPU (Intel Speed Shift EPP - Zero Stuttering)"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(0x0003), // BS_AUTOCHECKBOX = 0x0003
        s(36), s(346), s(660), s(24),
        parent, HMENU(ID_CHK_GOVERNOR_ENABLED as *mut _), hinstance, None,
    )?;
    SendMessageW(h_gov_chk, WM_SETFONT, WPARAM(font_btn.0 as usize), LPARAM(1));

    // Nhãn: Nhiệt độ CPU mục tiêu
    let h_gov_lbl = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("STATIC"),
        w!("Nhiệt độ CPU mục tiêu:"),
        WS_CHILD | WS_VISIBLE,
        s(36), s(374), s(210), s(22),
        parent, HMENU::default(), hinstance, None,
    )?;
    SendMessageW(h_gov_lbl, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

    // Edit: Nhiệt độ mục tiêu
    let h_gov_edit = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("EDIT"),
        w!("80"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(0x2000 | 0x0001), // ES_NUMBER | ES_CENTER
        s(252), s(372), s(52), s(24),
        parent, HMENU(ID_EDIT_GOVERNOR_TARGET as *mut _), hinstance, None,
    )?;
    SendMessageW(h_gov_edit, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

    let h_gov_hint = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("STATIC"),
        w!("°C   (Khuyên dùng: 75°C - 85°C. Mượt mà, giữ Boost)"),
        WS_CHILD | WS_VISIBLE,
        s(312), s(374), s(395), s(22),
        parent, HMENU::default(), hinstance, None,
    )?;
    SendMessageW(h_gov_hint, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

    // --- PRESETS NHANH ---
    let h_pre_lbl = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("STATIC"),
        w!("Mẫu thiết lập:"),
        WS_CHILD | WS_VISIBLE,
        s(20), s(418), s(125), s(24),
        parent, HMENU::default(), hinstance, None,
    )?;
    SendMessageW(h_pre_lbl, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

    let btn_preset_def = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Mặc định (Default)"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        s(155), s(414), s(170), s(28),
        parent, HMENU(ID_BTN_PRESET_DEFAULT as *mut _), hinstance, None,
    )?;
    SendMessageW(btn_preset_def, WM_SETFONT, WPARAM(font_btn.0 as usize), LPARAM(1));

    let btn_preset_game = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Tản nhiệt cao (Gaming)"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        s(335), s(414), s(210), s(28),
        parent, HMENU(ID_BTN_PRESET_GAMING as *mut _), hinstance, None,
    )?;
    SendMessageW(btn_preset_game, WM_SETFONT, WPARAM(font_btn.0 as usize), LPARAM(1));

    let btn_preset_silent = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Yên tĩnh (Silent)"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        s(555), s(414), s(165), s(28),
        parent, HMENU(ID_BTN_PRESET_SILENT as *mut _), hinstance, None,
    )?;
    SendMessageW(btn_preset_silent, WM_SETFONT, WPARAM(font_btn.0 as usize), LPARAM(1));

    // --- NÚT HÀNH ĐỘNG DƯỚI CÙNG ---
    let btn_save = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Áp dụng && Lưu cấu hình"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(0x0001), // BS_DEFPUSHBUTTON
        s(20), s(452), s(224), s(34),
        parent, HMENU(ID_BTN_SAVE_APPLY as *mut _), hinstance, None,
    )?;
    SendMessageW(btn_save, WM_SETFONT, WPARAM(font_btn.0 as usize), LPARAM(1));

    let btn_open = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Mở file config.toml"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        s(258), s(452), s(224), s(34),
        parent, HMENU(ID_BTN_OPEN_CONFIG as *mut _), hinstance, None,
    )?;
    SendMessageW(btn_open, WM_SETFONT, WPARAM(font_btn.0 as usize), LPARAM(1));

    let btn_close = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("BUTTON"),
        w!("Đóng"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        s(496), s(452), s(224), s(34),
        parent, HMENU(ID_BTN_CLOSE as *mut _), hinstance, None,
    )?;
    SendMessageW(btn_close, WM_SETFONT, WPARAM(font_btn.0 as usize), LPARAM(1));

    // --- KHUNG HIỂN THỊ THÔNG SỐ THỜI GIAN THỰC (LIVE TELEMETRY) ---
    let h_telemetry = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("STATIC"),
        w!("Trạng thái phần cứng: Đang nạp cảm biến..."),
        WS_CHILD | WS_VISIBLE,
        s(20), s(498), s(700), s(26),
        parent, HMENU::default(), hinstance, None,
    )?;
    SendMessageW(h_telemetry, WM_SETFONT, WPARAM(font_main.0 as usize), LPARAM(1));

    Ok(DialogControls {
        cpu_temp_edits,
        cpu_speed_edits,
        gpu_temp_edits,
        gpu_speed_edits,
        gov_enabled_chk: h_gov_chk,
        gov_target_edit: h_gov_edit,
        telemetry_lbl: h_telemetry,
        last_telemetry: String::new(),
        state,
        cmd_sender,
        font_main,
        font_btn,
    })
}

/// Nạp dữ liệu cấu hình từ file vào các ô Edit
unsafe fn load_config_into_controls(controls_ptr: *mut DialogControls) {
    if controls_ptr.is_null() { return; }
    let controls = &*controls_ptr;

    let config = Config::load_or_default(crate::config::get_config_path());

    // Điền CPU points (tối đa 4 điểm)
    for (i, p) in config.cpu_curve.iter().take(4).enumerate() {
        set_edit_number(controls.cpu_temp_edits[i], p.temp);
        set_edit_number(controls.cpu_speed_edits[i], p.speed as u32);
    }

    // Điền GPU points (tối đa 3 điểm)
    for (i, p) in config.gpu_curve.iter().take(3).enumerate() {
        set_edit_number(controls.gpu_temp_edits[i], p.temp);
        set_edit_number(controls.gpu_speed_edits[i], p.speed as u32);
    }

    // Nạp Thermal Governor
    let check_val = if config.thermal_governor.enabled { BST_CHECKED } else { BST_UNCHECKED };
    let _ = SendMessageW(controls.gov_enabled_chk, BM_SETCHECK, WPARAM(check_val), LPARAM(0));
    set_edit_number(controls.gov_target_edit, config.thermal_governor.target_cpu_temp);
}

/// Thiết lập số vào Edit Control
unsafe fn set_edit_number(hwnd: HWND, val: u32) {
    let s = format!("{}\0", val);
    let wide: Vec<u16> = s.encode_utf16().collect();
    let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
}

/// Đọc số nguyên từ Edit Control
unsafe fn get_edit_number(hwnd: HWND) -> u32 {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 { return 0; }
    let mut buf = vec![0u16; (len + 1) as usize];
    let read_len = GetWindowTextW(hwnd, &mut buf);
    if read_len <= 0 { return 0; }
    let s = String::from_utf16_lossy(&buf[..read_len as usize]);
    s.trim().parse::<u32>().unwrap_or(0)
}

/// Áp dụng mẫu Preset vào các ô nhập
unsafe fn apply_preset_to_controls(controls: &DialogControls, preset_type: &str) {
    let (cpu_pts, gpu_pts) = match preset_type {
        "gaming" => (
            vec![(40, 35), (55, 65), (70, 85), (80, 100)],
            vec![(45, 45), (60, 75), (75, 100)],
        ),
        "silent" => (
            vec![(45, 15), (65, 35), (80, 65), (90, 100)],
            vec![(50, 20), (70, 45), (85, 100)],
        ),
        _ => ( // Default
            vec![(40, 20), (60, 50), (75, 80), (85, 100)],
            vec![(45, 25), (65, 60), (80, 100)],
        ),
    };

    for (i, (t, s)) in cpu_pts.iter().enumerate() {
        set_edit_number(controls.cpu_temp_edits[i], *t);
        set_edit_number(controls.cpu_speed_edits[i], *s);
    }

    for (i, (t, s)) in gpu_pts.iter().enumerate() {
        set_edit_number(controls.gpu_temp_edits[i], *t);
        set_edit_number(controls.gpu_speed_edits[i], *s);
    }
}

/// Lưu các giá trị hiện tại trên giao diện vào file `config.toml` và kích hoạt Hot-Reload
unsafe fn save_and_apply_curve(hwnd: HWND, controls: &DialogControls) {
    let mut config = Config::load_or_default(crate::config::get_config_path());

    let mut cpu_curve = Vec::new();
    for i in 0..4 {
        let temp = get_edit_number(controls.cpu_temp_edits[i]);
        let speed = get_edit_number(controls.cpu_speed_edits[i]);
        if temp > 0 {
            cpu_curve.push(CurvePoint::new(temp.clamp(20, 110), (speed as u8).min(100)));
        }
    }

    let mut gpu_curve = Vec::new();
    for i in 0..3 {
        let temp = get_edit_number(controls.gpu_temp_edits[i]);
        let speed = get_edit_number(controls.gpu_speed_edits[i]);
        if temp > 0 {
            gpu_curve.push(CurvePoint::new(temp.clamp(20, 110), (speed as u8).min(100)));
        }
    }

    if cpu_curve.is_empty() || gpu_curve.is_empty() {
        MessageBoxW(
            hwnd,
            w!("Vui lòng nhập ít nhất 1 mốc nhiệt độ hợp lệ cho cả CPU và GPU!"),
            w!("Dữ liệu chưa hoàn thiện"),
            MB_OK | MB_ICONWARNING,
        );
        return;
    }

    // Đọc trạng thái Thermal Governor từ giao diện
    let gov_checked = SendMessageW(controls.gov_enabled_chk, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 as usize == BST_CHECKED;
    let mut gov_target = get_edit_number(controls.gov_target_edit);
    if gov_target == 0 {
        gov_target = 80;
    }

    config.thermal_governor.enabled = gov_checked;
    config.thermal_governor.target_cpu_temp = gov_target.clamp(50, 100);

    config.cpu_curve = cpu_curve;
    config.gpu_curve = gpu_curve;
    config.validate_and_sanitize();

    match config.save() {
        Ok(_) => {
            // Cập nhật governor status trên TrayState
            if let Some(ref st) = controls.state {
                let (_, _, curr_epp) = st.get_governor_info();
                st.update_governor_status(config.thermal_governor.enabled, config.thermal_governor.target_cpu_temp, curr_epp);
            }

            // Gửi lệnh hot-reload tới Worker thread điều khiển quạt
            let _ = controls.cmd_sender.send(TrayCommand::ReloadConfig);

            MessageBoxW(
                hwnd,
                w!("Cấu hình Fan Curve & Khóa nhiệt độ CPU (Smooth EPP) đã được lưu vào config.toml và áp dụng ngay lập tức!"),
                w!("Lưu thành công"),
                MB_OK | MB_ICONINFORMATION,
            );
            let _ = DestroyWindow(hwnd);
        }
        Err(err) => {
            let msg = format!("Lỗi khi lưu config.toml: {}\0", err);
            let wide: Vec<u16> = msg.encode_utf16().collect();
            MessageBoxW(
                hwnd,
                PCWSTR(wide.as_ptr()),
                w!("Lỗi lưu tệp"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

/// Window Procedure cho cửa sổ cấu hình Fan Curve
unsafe extern "system" fn dialog_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as usize;
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DialogControls;
            if let Some(controls) = ptr.as_ref() {
                match id {
                    ID_BTN_PRESET_DEFAULT => {
                        apply_preset_to_controls(controls, "default");
                    }
                    ID_BTN_PRESET_GAMING => {
                        apply_preset_to_controls(controls, "gaming");
                    }
                    ID_BTN_PRESET_SILENT => {
                        apply_preset_to_controls(controls, "silent");
                    }
                    ID_BTN_SAVE_APPLY => {
                        save_and_apply_curve(hwnd, controls);
                    }
                    ID_BTN_OPEN_CONFIG => {
                        let _ = std::process::Command::new("notepad.exe").arg(crate::config::get_config_path()).spawn();
                    }
                    ID_BTN_CLOSE => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            // Đảm bảo chữ trên STATIC label rõ nét, xóa sạch nền cũ tránh lỗi vệt đen ma (text ghosting / smearing)
            let hdc = HDC(wparam.0 as *mut _);
            SetBkMode(hdc, OPAQUE);
            SetBkColor(hdc, COLORREF(GetSysColor(COLOR_BTNFACE)));
            SetTextColor(hdc, COLORREF(0x001A1A1A));
            let brush = GetSysColorBrush(COLOR_BTNFACE);
            LRESULT(brush.0 as isize)
        }
        WM_TIMER => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DialogControls;
            if let Some(controls) = ptr.as_mut() {
                if let Some(ref st) = controls.state {
                    let (cpu_t, cpu_s, gpu_t, gpu_s, cpu_rpm, gpu_rpm) = st.get_status_rpm();
                    let (gov_en, gov_tgt, gov_epp) = st.get_governor_info();
                    let gov_str = if gov_en {
                        if gov_epp == 0 {
                            format!(" | Thermal Target: {}°C (EPP: 0 - Boost)", gov_tgt)
                        } else {
                            format!(" | Thermal Target: {}°C (EPP: {})", gov_tgt, gov_epp)
                        }
                    } else {
                        " | Thermal Target: OFF".to_string()
                    };
                    let status_str = if cpu_rpm > 0 || gpu_rpm > 0 {
                        format!(
                            "Đang hoạt động: CPU: {}°C ({} RPM - {}%)  |  GPU: {}°C ({} RPM - {}%){}\0",
                            cpu_t, cpu_rpm, cpu_s, gpu_t, gpu_rpm, gpu_s, gov_str
                        )
                    } else {
                        format!(
                            "Đang hoạt động: CPU: {}°C ({}%)  |  GPU: {}°C ({}%){}\0",
                            cpu_t, cpu_s, gpu_t, gpu_s, gov_str
                        )
                    };
                    if status_str != controls.last_telemetry {
                        controls.last_telemetry = status_str.clone();
                        let wide: Vec<u16> = status_str.encode_utf16().collect();
                        let _ = InvalidateRect(controls.telemetry_lbl, None, true);
                        let _ = SetWindowTextW(controls.telemetry_lbl, PCWSTR(wide.as_ptr()));
                    }
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(hwnd, 101);
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DialogControls;
            if !ptr.is_null() {
                let controls = Box::from_raw(ptr);
                let _ = DeleteObject(controls.font_main);
                let _ = DeleteObject(controls.font_btn);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_curve_dialog_dpi_and_font_helpers() {
        // 96 DPI
        assert_eq!(scale_dpi(560, 96), 560);
        assert_eq!(scale_dpi(590, 96), 590);
        assert_eq!(scale_dpi(640, 96), 640);
        assert_eq!(scale_dpi(720, 96), 720);
        assert_eq!(scale_dpi(740, 96), 740);
        assert_eq!(scale_dpi(550, 96), 550);
        assert_eq!(scale_dpi(600, 96), 600);
        assert_eq!(scale_dpi(168, 96), 168);
        assert_eq!(scale_dpi(200, 96), 200);
        assert_eq!(font_height_for_pt(12.0, 96), -16);
        assert_eq!(font_height_for_pt(12.5, 96), -17);
        assert_eq!(font_height_for_pt(13.0, 96), -17);

        // 120 DPI (125% scaling)
        assert_eq!(scale_dpi(560, 120), 700);
        assert_eq!(scale_dpi(590, 120), 738);
        assert_eq!(scale_dpi(640, 120), 800);
        assert_eq!(scale_dpi(720, 120), 900);
        assert_eq!(scale_dpi(740, 120), 925);
        assert_eq!(scale_dpi(550, 120), 688);
        assert_eq!(scale_dpi(600, 120), 750);
        assert_eq!(scale_dpi(168, 120), 210);
        assert_eq!(scale_dpi(200, 120), 250);
        assert_eq!(font_height_for_pt(12.0, 120), -20);
        assert_eq!(font_height_for_pt(12.5, 120), -21);
        assert_eq!(font_height_for_pt(13.0, 120), -22);
    }
}
