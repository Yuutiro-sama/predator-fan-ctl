//! Acer Predator Temperature, Power & Clock History Chart Window (Pure Win32 GDI Dark Mode)
//!
//! Cửa sổ hiển thị biểu đồ phần cứng đa chế độ CPU và GPU trong 1 giờ gần nhất:
//! - Giao diện Dark Mode chuẩn Predator Theme (#121417).
//! - Hoàn toàn bằng Win32 GDI thuần, Double-Buffering chống nhấp nháy (Flicker-Free).
//! - Thanh chọn 3 chế độ (Mode Tabs) với hit-test chuột và phím tắt (1, 2, 3):
//!   1. [ 🌡️ Nhiệt độ & Quạt ] (30°C - 100°C, cảnh báo 85°C)
//!   2. [ ⚡ Công suất (W) ] (0 - 150 W, tự co giãn 200 W, vạch 30 W)
//!   3. [ ⏱️ Xung nhịp (GHz) ] (0.0 - 5.5 GHz, vạch 1.0 GHz)
//! - Thanh trạng thái HUD thời gian thực (Live Infobar):
//!   "CPU: {temp}°C | {power:.1}W | {clock:.2} GHz   •   GPU: {temp}°C | {power:.1}W | {clock:.2} GHz"
//! - Trục X: 1 giờ (3600 giây) chia mốc mỗi 10 phút (-60m .. Now).
//! - Đường biểu diễn CPU (Cyan #00E5FF) và GPU (Orange #FF9100) độ dày 2px.
//! - Tương tác chuột: Crosshair dọc và Tooltip hiển thị thời gian và thông số chi tiết theo từng chế độ.
//! - Tự động cập nhật mỗi 1 giây qua WM_TIMER.
//! - Quản lý Singleton: Giữ cửa sổ duy nhất, ẩn khi đóng (WM_CLOSE) và khôi phục khi mở lại.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use windows::core::{w, PCWSTR, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::telemetry::{TelemetryHistory, TelemetrySample};

/// Handle cửa sổ biểu đồ đang mở (Singleton)
static ACTIVE_CHART_HWND: AtomicIsize = AtomicIsize::new(0);

const TIMER_REFRESH: usize = 1;
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Chế độ hiển thị biểu đồ phần cứng Predator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ChartMode {
    #[default]
    TempAndFan = 0, // 🌡️ Nhiệt độ (°C) & Quạt (%)
    Power = 1,      // ⚡ Công suất (W)
    Clock = 2,      // ⏱️ Xung nhịp (GHz)
}

impl ChartMode {
    pub const fn from_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(Self::TempAndFan),
            1 => Some(Self::Power),
            2 => Some(Self::Clock),
            _ => None,
        }
    }

    pub const fn to_index(self) -> usize {
        self as usize
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::TempAndFan => "🌡️ Nhiệt độ & Quạt",
            Self::Power => "⚡ Công suất (W)",
            Self::Clock => "⏱️ Xung nhịp (GHz)",
        }
    }
}

/// Màu sắc theo bảng màu Predator Dark Theme
#[inline]
pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

const COLOR_BG: COLORREF = rgb(18, 20, 23);           // #121417
const COLOR_PLOT_BG: COLORREF = rgb(22, 25, 30);      // #16191E
const COLOR_BORDER: COLORREF = rgb(52, 59, 72);       // #343B48
const COLOR_GRID: COLORREF = rgb(40, 44, 52);         // #282C34
const COLOR_WARN_85: COLORREF = rgb(255, 69, 58);     // #FF453A (Đỏ cảnh báo rực rỡ)
const COLOR_CPU: COLORREF = rgb(0, 229, 255);         // #00E5FF (Cyan sáng rực)
const COLOR_GPU: COLORREF = rgb(255, 145, 0);         // #FF9100 (Orange sáng rực)
const COLOR_TEXT_MUTED: COLORREF = rgb(176, 190, 208); // #B0BED0 (Chữ phụ tương phản cao)
const COLOR_TEXT_LIGHT: COLORREF = rgb(240, 244, 248); // #F0F4F8 (Trắng sáng nét căng)
const COLOR_CROSSHAIR: COLORREF = rgb(120, 130, 150); // #788296
const COLOR_TOOLTIP_BG: COLORREF = rgb(26, 29, 36);   // #1A1D24
const COLOR_TOOLTIP_BORDER: COLORREF = rgb(70, 78, 92); // #464E5C

// Bảng màu cho thanh Tab
const COLOR_TAB_BG_ACTIVE: COLORREF = rgb(38, 45, 56);     // #262D38
const COLOR_TAB_BG_INACTIVE: COLORREF = rgb(22, 25, 30);   // #16191E
const COLOR_TAB_BORDER: COLORREF = rgb(65, 74, 90);        // #414A5A
const COLOR_TAB_INDICATOR: COLORREF = rgb(0, 229, 255);    // #00E5FF

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

/// Vị trí tọa độ của từng nút Tab thích ứng với DPI (Hit-Testing & Vẽ)
pub fn get_tab_rect_dpi(mode: ChartMode, dpi: u32) -> RECT {
    if dpi == 96 {
        return match mode {
            ChartMode::TempAndFan => RECT { left: 20, top: 8, right: 180, bottom: 34 },
            ChartMode::Power => RECT { left: 186, top: 8, right: 326, bottom: 34 },
            ChartMode::Clock => RECT { left: 332, top: 8, right: 482, bottom: 34 },
        };
    }
    let top = scale_dpi(8, dpi);
    let tab_h = scale_dpi(36, dpi);
    let bottom = top + tab_h;

    let left_base = scale_dpi(20, dpi);
    let gap = scale_dpi(6, dpi);

    let w0 = scale_dpi(175, dpi);
    let w1 = scale_dpi(155, dpi);
    let w2 = scale_dpi(165, dpi);

    match mode {
        ChartMode::TempAndFan => RECT {
            left: left_base,
            top,
            right: left_base + w0,
            bottom,
        },
        ChartMode::Power => {
            let left = left_base + w0 + gap;
            RECT {
                left,
                top,
                right: left + w1,
                bottom,
            }
        }
        ChartMode::Clock => {
            let left = left_base + w0 + gap + w1 + gap;
            RECT {
                left,
                top,
                right: left + w2,
                bottom,
            }
        }
    }
}

/// Vị trí tọa độ của từng nút Tab (tiêu chuẩn 96 DPI - Tương thích ngược Unit Tests)
pub fn get_tab_rect(mode: ChartMode) -> RECT {
    get_tab_rect_dpi(mode, 96)
}

/// Hit-testing tìm tab khi người dùng click chuột thích ứng DPI
pub fn hit_test_tab_dpi(x: i32, y: i32, dpi: u32) -> Option<ChartMode> {
    for &mode in &[ChartMode::TempAndFan, ChartMode::Power, ChartMode::Clock] {
        let r = get_tab_rect_dpi(mode, dpi);
        if x >= r.left && x <= r.right && y >= r.top && y <= r.bottom {
            return Some(mode);
        }
    }
    None
}

/// Hit-testing tìm tab khi người dùng click chuột (tiêu chuẩn 96 DPI - Tương thích ngược)
pub fn hit_test_tab(x: i32, y: i32) -> Option<ChartMode> {
    hit_test_tab_dpi(x, y, 96)
}

/// Trạng thái dữ liệu nội bộ gắn với HWND
struct ChartWindowState {
    telemetry: Arc<Mutex<TelemetryHistory>>,
    mouse_pos: Option<POINT>,
    is_mouse_tracking: bool,
    active_mode: ChartMode,
}

/// Mở hoặc đưa cửa sổ biểu đồ nhiệt độ lên phía trước (Singleton)
pub fn show_or_focus_chart_window(telemetry: Arc<Mutex<TelemetryHistory>>) {
    let existing = ACTIVE_CHART_HWND.load(Ordering::Relaxed);
    if existing != 0 {
        let hwnd = HWND(existing as *mut _);
        unsafe {
            if IsWindow(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
                let _ = BringWindowToTop(hwnd);
                let _ = SetTimer(hwnd, TIMER_REFRESH, 1000, None);
                let _ = InvalidateRect(hwnd, None, false);
                return;
            }
        }
    }

    thread::spawn(move || {
        if let Err(e) = run_chart_window(telemetry) {
            eprintln!("[ChartWindow] Lỗi khởi tạo cửa sổ biểu đồ phần cứng: {:?}", e);
        }
    });
}

/// Chạy vòng lặp thông điệp của cửa sổ biểu đồ
fn run_chart_window(telemetry: Arc<Mutex<TelemetryHistory>>) -> Result<()> {
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null())?;
        let class_name = w!("PredatorTempChartWndClass");

        let app_icon = crate::tray::load_embedded_app_icon();

        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(chart_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance.into(),
            hIcon: app_icon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
        };

        let _ = RegisterClassW(&wnd_class);

        // Kích thước mặc định co giãn theo DPI (860 x 560 ở 96 DPI, tự co giãn ở 120 DPI)
        let hdc_screen = GetDC(HWND::default());
        let initial_dpi = GetDeviceCaps(hdc_screen, LOGPIXELSX) as u32;
        let _ = ReleaseDC(HWND::default(), hdc_screen);
        let initial_dpi = if initial_dpi > 0 { initial_dpi } else { 96 };

        let width = scale_dpi(860, initial_dpi);
        let height = scale_dpi(560, initial_dpi);
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let pos_x = (screen_w - width) / 2;
        let pos_y = (screen_h - height) / 2;

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("Acer Predator - Lịch sử phần cứng thời gian thực (1 Giờ)"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            pos_x,
            pos_y,
            width,
            height,
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        )?;

        if !app_icon.is_invalid() {
            let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_BIG as _), LPARAM(app_icon.0 as _));
            let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_SMALL as _), LPARAM(app_icon.0 as _));
        }

        ACTIVE_CHART_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

        let state = Box::new(ChartWindowState {
            telemetry,
            mouse_pos: None,
            is_mouse_tracking: false,
            active_mode: ChartMode::TempAndFan,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);

        let _ = SetTimer(hwnd, TIMER_REFRESH, 1000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        ACTIVE_CHART_HWND.store(0, Ordering::Relaxed);
        Ok(())
    }
}

/// Helper tạo HFONT Segoe UI DPI-Aware chuẩn Em-Height (-MulDiv(point_size, dpi, 72))
unsafe fn create_chart_dpi_font(dpi: u32, point_size: f32, weight: i32) -> HFONT {
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

/// Helper tạo HFONT Segoe UI (96 DPI tương thích)
#[allow(dead_code)]
unsafe fn create_chart_font(height: i32, weight: i32) -> HFONT {
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

/// Helper vẽ chuỗi văn bản với TextOutW
unsafe fn draw_text_line(hdc: HDC, x: i32, y: i32, text: &str, color: COLORREF) {
    SetTextColor(hdc, color);
    let wide: Vec<u16> = text.encode_utf16().collect();
    let _ = TextOutW(hdc, x, y, &wide);
}

// ============================================================================
// CÁC HÀM TÍNH TOÁN ÁNH XẠ TỌA ĐỘ (COORDINATE MAPPING & SCALING)
// ============================================================================

/// Ánh xạ thời gian (giây) sang tọa độ X trên Plot Area
pub fn map_time_x(timestamp_secs: u64, newest_ts: u64, plot_left: i32, plot_right: i32) -> Option<i32> {
    let k = newest_ts.saturating_sub(timestamp_secs);
    if k > 3600 {
        return None;
    }
    let plot_width = (plot_right - plot_left) as f32;
    let x = plot_right as f32 - ((k as f32 / 3600.0) * plot_width);
    Some((x.round() as i32).clamp(plot_left, plot_right))
}

/// Ánh xạ nhiệt độ (°C: 30°C - 100°C) sang tọa độ Y
pub fn map_temp_y(temp: u8, plot_top: i32, plot_bottom: i32) -> i32 {
    let plot_height = (plot_bottom - plot_top) as f32;
    let ratio = (temp.clamp(30, 100) - 30) as f32 / 70.0;
    plot_bottom - (ratio * plot_height).round() as i32
}

/// Tính toán thang đo trục Y công suất: mặc định 150W, tự động co giãn 200W nếu có đỉnh > 150W
pub fn calculate_power_scale(samples: &[TelemetrySample]) -> f32 {
    let mut peak = 0.0f32;
    for s in samples {
        let cpu_w = s.cpu_power_w();
        let gpu_w = s.gpu_power_w();
        if cpu_w > peak {
            peak = cpu_w;
        }
        if gpu_w > peak {
            peak = gpu_w;
        }
    }
    if peak > 150.0 {
        200.0
    } else {
        150.0
    }
}

/// Ánh xạ công suất (W: 0 - 150W hoặc 0 - 200W) sang tọa độ Y
pub fn map_power_y(power_w: f32, max_y: f32, plot_top: i32, plot_bottom: i32) -> i32 {
    let plot_height = (plot_bottom - plot_top) as f32;
    let clamped = power_w.clamp(0.0, max_y);
    let ratio = clamped / max_y;
    plot_bottom - (ratio * plot_height).round() as i32
}

/// Ánh xạ xung nhịp (GHz: 0.0 - 5.5 GHz) sang tọa độ Y
pub fn map_clock_y(clock_ghz: f32, plot_top: i32, plot_bottom: i32) -> i32 {
    let plot_height = (plot_bottom - plot_top) as f32;
    let clamped = clock_ghz.clamp(0.0, 5.5);
    let ratio = clamped / 5.5;
    plot_bottom - (ratio * plot_height).round() as i32
}

// ============================================================================
// HÀM VẼ GDI CHÍNH (DOUBLE-BUFFERING RENDER ENGINE)
// ============================================================================

/// Vẽ toàn bộ nội dung biểu đồ lên Memory DC (Double-Buffering & DPI-Aware)
unsafe fn paint_chart(hdc: HDC, width: i32, height: i32, state: &ChartWindowState, dpi: u32) {
    // 1. Tô nền toàn bộ cửa sổ #121417
    let full_rect = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let bg_brush = CreateSolidBrush(COLOR_BG);
    FillRect(hdc, &full_rect, bg_brush);
    let _ = DeleteObject(bg_brush);

    SetBkMode(hdc, TRANSPARENT);

    // Nạp dữ liệu lịch sử từ bộ đệm Telemetry
    let samples = if let Ok(hist) = state.telemetry.lock() {
        hist.iter_chronological()
    } else {
        Vec::new()
    };

    // 2. Vẽ Thanh chọn chế độ (Mode Tabs) trên đầu cửa sổ - Font 13 pt Semi-Bold
    let tab_font = create_chart_dpi_font(dpi, 13.0, FW_SEMIBOLD.0 as i32);
    let old_font = SelectObject(hdc, tab_font);

    let tab_border_pen = CreatePen(PS_SOLID, 1, COLOR_TAB_BORDER);
    let tab_indicator_pen = CreatePen(PS_SOLID, 2, COLOR_TAB_INDICATOR);
    let old_pen = SelectObject(hdc, tab_border_pen);

    for &mode in &[ChartMode::TempAndFan, ChartMode::Power, ChartMode::Clock] {
        let r = get_tab_rect_dpi(mode, dpi);
        let is_active = state.active_mode == mode;

        // Nền nút tab
        let tab_brush = CreateSolidBrush(if is_active {
            COLOR_TAB_BG_ACTIVE
        } else {
            COLOR_TAB_BG_INACTIVE
        });
        FillRect(hdc, &r, tab_brush);
        let _ = DeleteObject(tab_brush);

        // Viền nút tab
        let _ = SelectObject(hdc, tab_border_pen);
        let border_pts = [
            POINT { x: r.left, y: r.top },
            POINT { x: r.right, y: r.top },
            POINT { x: r.right, y: r.bottom },
            POINT { x: r.left, y: r.bottom },
            POINT { x: r.left, y: r.top },
        ];
        let _ = Polyline(hdc, &border_pts);

        // Đường gạch chân điểm nhấn (Indicator line) cho tab đang chọn
        if is_active {
            let _ = SelectObject(hdc, tab_indicator_pen);
            let indicator_pts = [
                POINT { x: r.left + 2, y: r.bottom - 1 },
                POINT { x: r.right - 2, y: r.bottom - 1 },
            ];
            let _ = Polyline(hdc, &indicator_pts);
        }

        // Nhãn văn bản của Tab căn giữa hoàn hảo
        let label_color = if is_active {
            COLOR_CPU
        } else {
            COLOR_TEXT_MUTED
        };
        SetTextColor(hdc, label_color);
        let mut text_rect = RECT {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom - 2,
        };
        let mut wide_label: Vec<u16> = mode.label().encode_utf16().collect();
        let _ = DrawTextW(
            hdc,
            &mut wide_label,
            &mut text_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }

    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(tab_indicator_pen);
    let _ = DeleteObject(tab_border_pen);

    // 3. Thanh trạng thái HUD thời gian thực (Live Infobar) - Font 12.5 pt Semi-Bold
    let tab_bottom = get_tab_rect_dpi(ChartMode::TempAndFan, dpi).bottom;
    let hud_top = tab_bottom + scale_dpi(8, dpi);
    let hud_h = scale_dpi(28, dpi);
    let hud_font = create_chart_dpi_font(dpi, 12.5, FW_SEMIBOLD.0 as i32);
    let _ = SelectObject(hdc, hud_font);
    let hud_x = scale_dpi(20, dpi);
    let hud_text_y = hud_top + (hud_h - (12.5 * dpi as f32 / 72.0).round() as i32) / 2;

    match samples.last() {
        Some(s) => {
            let cpu_hud = format!(
                "CPU: {}°C | {:.1}W | {:.2} GHz",
                s.cpu_temp,
                s.cpu_power_w(),
                s.cpu_ghz()
            );
            let gpu_hud = format!(
                "GPU: {}°C | {:.1}W | {:.2} GHz",
                s.gpu_temp,
                s.gpu_power_w(),
                s.gpu_ghz()
            );

            let wide_cpu: Vec<u16> = cpu_hud.encode_utf16().collect();
            let mut size_cpu = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &wide_cpu, &mut size_cpu);

            let bullet = "   •   ";
            let wide_bullet: Vec<u16> = bullet.encode_utf16().collect();
            let mut size_bullet = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &wide_bullet, &mut size_bullet);

            let x_bullet = hud_x + size_cpu.cx;
            let x_gpu = x_bullet + size_bullet.cx;

            draw_text_line(hdc, hud_x, hud_text_y, &cpu_hud, COLOR_CPU);
            draw_text_line(hdc, x_bullet, hud_text_y, bullet, COLOR_TEXT_LIGHT);
            draw_text_line(hdc, x_gpu, hud_text_y, &gpu_hud, COLOR_GPU);
        }
        None => {
            draw_text_line(
                hdc,
                hud_x,
                hud_text_y,
                "CPU: --°C | --.-W | -.-- GHz   •   GPU: --°C | --.-W | -.-- GHz (Đang thu thập dữ liệu...)",
                COLOR_TEXT_MUTED,
            );
        }
    }

    let _ = DeleteObject(hud_font);

    // 4. Xác định vùng đồ thị (Plot Area) thích ứng DPI
    let plot_left = scale_dpi(66, dpi);
    let plot_top = hud_top + hud_h + scale_dpi(10, dpi);
    let plot_right = width - scale_dpi(25, dpi);
    let plot_bottom = height - scale_dpi(40, dpi);

    let plot_width = plot_right - plot_left;
    let plot_height = plot_bottom - plot_top;

    if plot_width <= 50 || plot_height <= 50 {
        let _ = SelectObject(hdc, old_font);
        let _ = DeleteObject(tab_font);
        return;
    }

    // Tô nền vùng Plot Area #16191E
    let plot_rect = RECT {
        left: plot_left,
        top: plot_top,
        right: plot_right,
        bottom: plot_bottom,
    };
    let plot_brush = CreateSolidBrush(COLOR_PLOT_BG);
    FillRect(hdc, &plot_rect, plot_brush);
    let _ = DeleteObject(plot_brush);

    // Font nhãn Trục Y - 11.5 pt
    let y_label_font = create_chart_dpi_font(dpi, 11.5, FW_MEDIUM.0 as i32);
    let _ = SelectObject(hdc, y_label_font);

    let grid_pen = CreatePen(PS_SOLID, 1, COLOR_GRID);
    let old_pen = SelectObject(hdc, grid_pen);

    // 5. Vẽ Trục Y theo từng Chế độ (Mode-Specific Y Axis)
    let power_scale = calculate_power_scale(&samples);

    match state.active_mode {
        ChartMode::TempAndFan => {
            // Trục Y: Nhiệt độ 30°C đến 100°C (bước 10°C)
            for temp in (30..=100).step_by(10) {
                let y = map_temp_y(temp as u8, plot_top, plot_bottom);

                // Đường lưới ngang
                let pts = [POINT { x: plot_left, y }, POINT { x: plot_right, y }];
                let _ = Polyline(hdc, &pts);

                // Nhãn nhiệt độ (°C) căn sát mép trái có đệm chuẩn
                let label = format!("{:3}°C", temp);
                let wide_lbl: Vec<u16> = label.encode_utf16().collect();
                let mut sz_lbl = windows::Win32::Foundation::SIZE::default();
                let _ = GetTextExtentPoint32W(hdc, &wide_lbl, &mut sz_lbl);
                let lbl_x = plot_left - sz_lbl.cx - scale_dpi(6, dpi);
                let lbl_y = y - (sz_lbl.cy / 2);
                draw_text_line(hdc, lbl_x, lbl_y, &label, COLOR_TEXT_LIGHT);
            }

            // Đường cảnh báo 85°C (Nét đứt màu đỏ rực rỡ)
            let warn_y = map_temp_y(85, plot_top, plot_bottom);
            let warn_pen = CreatePen(PS_DASH, 1, COLOR_WARN_85);
            let _ = SelectObject(hdc, warn_pen);
            let warn_pts = [POINT { x: plot_left, y: warn_y }, POINT { x: plot_right, y: warn_y }];
            let _ = Polyline(hdc, &warn_pts);
            let _ = DeleteObject(warn_pen);

            let warn_font = create_chart_dpi_font(dpi, 11.0, FW_BOLD.0 as i32);
            let _ = SelectObject(hdc, warn_font);
            let warn_text = "85°C Warning";
            let wide_w: Vec<u16> = warn_text.encode_utf16().collect();
            let mut sz_w = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(hdc, &wide_w, &mut sz_w);
            draw_text_line(
                hdc,
                plot_right - sz_w.cx - scale_dpi(10, dpi),
                warn_y - sz_w.cy - scale_dpi(2, dpi),
                warn_text,
                COLOR_WARN_85,
            );
            let _ = DeleteObject(warn_font);
            let _ = SelectObject(hdc, y_label_font);
            let _ = SelectObject(hdc, grid_pen);
        }
        ChartMode::Power => {
            // Trục Y: Công suất 0 - 150 W (hoặc 200W nếu vượt đỉnh 150W), vạch chia mỗi 30 W
            let max_w = power_scale as i32;
            let mut marks: Vec<i32> = (0..=max_w).step_by(30).collect();
            if *marks.last().unwrap_or(&0) < max_w {
                marks.push(max_w);
            }

            for &w_val in &marks {
                let y = map_power_y(w_val as f32, power_scale, plot_top, plot_bottom);

                // Đường lưới ngang
                let pts = [POINT { x: plot_left, y }, POINT { x: plot_right, y }];
                let _ = Polyline(hdc, &pts);

                // Nhãn công suất (W)
                let label = format!("{:3} W", w_val);
                let wide_lbl: Vec<u16> = label.encode_utf16().collect();
                let mut sz_lbl = windows::Win32::Foundation::SIZE::default();
                let _ = GetTextExtentPoint32W(hdc, &wide_lbl, &mut sz_lbl);
                let lbl_x = plot_left - sz_lbl.cx - scale_dpi(6, dpi);
                let lbl_y = y - (sz_lbl.cy / 2);
                draw_text_line(hdc, lbl_x, lbl_y, &label, COLOR_TEXT_LIGHT);
            }
        }
        ChartMode::Clock => {
            // Trục Y: Xung nhịp 0.0 - 5.5 GHz, chia vạch mỗi 1.0 GHz
            let clock_marks: [f32; 7] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 5.5];
            for &ghz in &clock_marks {
                let y = map_clock_y(ghz, plot_top, plot_bottom);

                // Đường lưới ngang
                let pts = [POINT { x: plot_left, y }, POINT { x: plot_right, y }];
                let _ = Polyline(hdc, &pts);

                // Nhãn xung nhịp (GHz)
                let label = format!("{:.1}G", ghz);
                let wide_lbl: Vec<u16> = label.encode_utf16().collect();
                let mut sz_lbl = windows::Win32::Foundation::SIZE::default();
                let _ = GetTextExtentPoint32W(hdc, &wide_lbl, &mut sz_lbl);
                let lbl_x = plot_left - sz_lbl.cx - scale_dpi(6, dpi);
                let lbl_y = y - (sz_lbl.cy / 2);
                draw_text_line(hdc, lbl_x, lbl_y, &label, COLOR_TEXT_LIGHT);
            }
        }
    }

    let _ = DeleteObject(y_label_font);

    // 6. Trục X: Thời gian 1 giờ (3600 giây), các mốc: -60m .. Now - Font 11 pt
    let x_label_font = create_chart_dpi_font(dpi, 11.0, FW_NORMAL.0 as i32);
    let _ = SelectObject(hdc, x_label_font);

    let time_marks: [(i32, &str); 7] = [
        (60, "-60m"),
        (50, "-50m"),
        (40, "-40m"),
        (30, "-30m"),
        (20, "-20m"),
        (10, "-10m"),
        (0, "Now"),
    ];

    for &(mins_ago, mark_text) in &time_marks {
        let fraction = (60 - mins_ago) as f32 / 60.0;
        let x = plot_left + (fraction * plot_width as f32).round() as i32;

        // Vạch lưới dọc mảnh
        let x_pts = [POINT { x, y: plot_top }, POINT { x, y: plot_bottom }];
        let _ = Polyline(hdc, &x_pts);

        // Vạch chia nhỏ (tick) dưới plot
        let tick_pts = [POINT { x, y: plot_bottom }, POINT { x, y: plot_bottom + scale_dpi(4, dpi) }];
        let _ = Polyline(hdc, &tick_pts);

        // Nhãn thời gian căn giữa
        let wide_m: Vec<u16> = mark_text.encode_utf16().collect();
        let mut sz_m = windows::Win32::Foundation::SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &wide_m, &mut sz_m);
        let lbl_x = x - (sz_m.cx / 2);
        let lbl_y = plot_bottom + scale_dpi(6, dpi);
        draw_text_line(hdc, lbl_x, lbl_y, mark_text, COLOR_TEXT_LIGHT);
    }

    let _ = DeleteObject(x_label_font);

    // Viền khung Plot Area
    let border_pen = CreatePen(PS_SOLID, 1, COLOR_BORDER);
    let _ = SelectObject(hdc, border_pen);
    let border_pts = [
        POINT { x: plot_left, y: plot_top },
        POINT { x: plot_right, y: plot_top },
        POINT { x: plot_right, y: plot_bottom },
        POINT { x: plot_left, y: plot_bottom },
        POINT { x: plot_left, y: plot_top },
    ];
    let _ = Polyline(hdc, &border_pts);
    let _ = DeleteObject(border_pen);

    // 7. Vẽ các đường biểu diễn (Polyline) theo từng Chế độ
    let mut mapped_samples: Vec<(i32, i32, i32, &TelemetrySample)> = Vec::with_capacity(samples.len());

    if samples.is_empty() {
        let empty_font = create_chart_dpi_font(dpi, 12.0, FW_NORMAL.0 as i32);
        let _ = SelectObject(hdc, empty_font);
        let empty_msg = "Chưa có dữ liệu cảm biến (đang thu thập...)";
        let wide_e: Vec<u16> = empty_msg.encode_utf16().collect();
        let mut sz_e = windows::Win32::Foundation::SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &wide_e, &mut sz_e);
        draw_text_line(
            hdc,
            plot_left + (plot_width - sz_e.cx) / 2,
            plot_top + (plot_height - sz_e.cy) / 2,
            empty_msg,
            COLOR_TEXT_MUTED,
        );
        let _ = DeleteObject(empty_font);
    } else {
        let newest_ts = samples.last().map(|s| s.timestamp_secs).unwrap_or(0);

        let mut cpu_pts: Vec<POINT> = Vec::with_capacity(samples.len());
        let mut gpu_pts: Vec<POINT> = Vec::with_capacity(samples.len());

        for s in &samples {
            if let Some(x) = map_time_x(s.timestamp_secs, newest_ts, plot_left, plot_right) {
                let (cpu_y, gpu_y) = match state.active_mode {
                    ChartMode::TempAndFan => (
                        map_temp_y(s.cpu_temp, plot_top, plot_bottom),
                        map_temp_y(s.gpu_temp, plot_top, plot_bottom),
                    ),
                    ChartMode::Power => (
                        map_power_y(s.cpu_power_w(), power_scale, plot_top, plot_bottom),
                        map_power_y(s.gpu_power_w(), power_scale, plot_top, plot_bottom),
                    ),
                    ChartMode::Clock => (
                        map_clock_y(s.cpu_ghz(), plot_top, plot_bottom),
                        map_clock_y(s.gpu_ghz(), plot_top, plot_bottom),
                    ),
                };

                cpu_pts.push(POINT { x, y: cpu_y });
                gpu_pts.push(POINT { x, y: gpu_y });

                mapped_samples.push((x, cpu_y, gpu_y, s));
            }
        }

        // Vẽ đường GPU (Cam #FF9100, 2px)
        let gpu_pen = CreatePen(PS_SOLID, 2, COLOR_GPU);
        let _ = SelectObject(hdc, gpu_pen);
        if gpu_pts.len() >= 2 {
            let _ = Polyline(hdc, &gpu_pts);
        } else if let Some(pt) = gpu_pts.first() {
            let _ = Ellipse(hdc, pt.x - 2, pt.y - 2, pt.x + 2, pt.y + 2);
        }
        let _ = DeleteObject(gpu_pen);

        // Vẽ đường CPU (Cyan #00E5FF, 2px)
        let cpu_pen = CreatePen(PS_SOLID, 2, COLOR_CPU);
        let _ = SelectObject(hdc, cpu_pen);
        if cpu_pts.len() >= 2 {
            let _ = Polyline(hdc, &cpu_pts);
        } else if let Some(pt) = cpu_pts.first() {
            let _ = Ellipse(hdc, pt.x - 2, pt.y - 2, pt.x + 2, pt.y + 2);
        }
        let _ = DeleteObject(cpu_pen);

        // Điểm chấm tròn hiện tại ở mép phải
        if let Some(last_cpu) = cpu_pts.last() {
            let cpu_brush = CreateSolidBrush(COLOR_CPU);
            let old_b = SelectObject(hdc, cpu_brush);
            let _ = Ellipse(hdc, last_cpu.x - 3, last_cpu.y - 3, last_cpu.x + 4, last_cpu.y + 4);
            let _ = SelectObject(hdc, old_b);
            let _ = DeleteObject(cpu_brush);
        }
        if let Some(last_gpu) = gpu_pts.last() {
            let gpu_brush = CreateSolidBrush(COLOR_GPU);
            let old_b = SelectObject(hdc, gpu_brush);
            let _ = Ellipse(hdc, last_gpu.x - 3, last_gpu.y - 3, last_gpu.x + 4, last_gpu.y + 4);
            let _ = SelectObject(hdc, old_b);
            let _ = DeleteObject(gpu_brush);
        }
    }

    // 8. Tương tác chuột (Crosshair & Tooltip) - Font 11.5 pt Semi-Bold
    if let Some(m_pos) = state.mouse_pos {
        if m_pos.x >= plot_left && m_pos.x <= plot_right && m_pos.y >= plot_top && m_pos.y <= plot_bottom {
            if let Some(&(closest_x, cpu_y, gpu_y, sample)) = mapped_samples.iter().min_by_key(|(x, _, _, _)| (x - m_pos.x).abs()) {
                // Đường gióng dọc Crosshair
                let cross_pen = CreatePen(PS_DOT, 1, COLOR_CROSSHAIR);
                let _ = SelectObject(hdc, cross_pen);
                let cross_pts = [
                    POINT { x: closest_x, y: plot_top },
                    POINT { x: closest_x, y: plot_bottom },
                ];
                let _ = Polyline(hdc, &cross_pts);
                let _ = DeleteObject(cross_pen);

                // Điểm giao điểm trên đường CPU
                let cpu_dot_brush = CreateSolidBrush(COLOR_CPU);
                let old_b = SelectObject(hdc, cpu_dot_brush);
                let _ = Ellipse(hdc, closest_x - 4, cpu_y - 4, closest_x + 5, cpu_y + 5);
                let _ = SelectObject(hdc, old_b);
                let _ = DeleteObject(cpu_dot_brush);

                // Điểm giao điểm trên đường GPU
                let gpu_dot_brush = CreateSolidBrush(COLOR_GPU);
                let old_b = SelectObject(hdc, gpu_dot_brush);
                let _ = Ellipse(hdc, closest_x - 4, gpu_y - 4, closest_x + 5, gpu_y + 5);
                let _ = SelectObject(hdc, old_b);
                let _ = DeleteObject(gpu_dot_brush);

                // Tính toán thông tin hiển thị Tooltip theo Chế độ hiện tại
                let newest_ts = samples.last().map(|s| s.timestamp_secs).unwrap_or(0);
                let k = newest_ts.saturating_sub(sample.timestamp_secs);
                let time_str = if k == 0 {
                    "Hiện tại (Now)".to_string()
                } else if k < 60 {
                    format!("-{} giây trước", k)
                } else {
                    format!("-{}m {:02}s trước", k / 60, k % 60)
                };

                let (line_cpu, line_gpu) = match state.active_mode {
                    ChartMode::TempAndFan => (
                        format!("CPU: {}°C (Quạt: {}%)", sample.cpu_temp, sample.cpu_fan_pct),
                        format!("GPU: {}°C (Quạt: {}%)", sample.gpu_temp, sample.gpu_fan_pct),
                    ),
                    ChartMode::Power => (
                        format!("CPU: {:.1} W (Quạt: {}%)", sample.cpu_power_w(), sample.cpu_fan_pct),
                        format!("GPU: {:.1} W (Quạt: {}%)", sample.gpu_power_w(), sample.gpu_fan_pct),
                    ),
                    ChartMode::Clock => (
                        format!("CPU: {:.2} GHz ({} MHz)", sample.cpu_ghz(), sample.cpu_mhz),
                        format!("GPU: {:.2} GHz ({} MHz)", sample.gpu_ghz(), sample.gpu_mhz),
                    ),
                };

                // Vẽ hộp Tooltip thích ứng DPI
                let tooltip_font = create_chart_dpi_font(dpi, 11.5, FW_SEMIBOLD.0 as i32);
                let _ = SelectObject(hdc, tooltip_font);

                let tt_w = scale_dpi(205, dpi);
                let tt_h = scale_dpi(86, dpi);
                let mut tt_x = closest_x + scale_dpi(15, dpi);
                let mut tt_y = m_pos.y - (tt_h / 2);

                if tt_x + tt_w > plot_right + scale_dpi(15, dpi) {
                    tt_x = closest_x - tt_w - scale_dpi(15, dpi);
                }
                if tt_y < plot_top + scale_dpi(5, dpi) {
                    tt_y = plot_top + scale_dpi(5, dpi);
                }
                if tt_y + tt_h > plot_bottom - scale_dpi(5, dpi) {
                    tt_y = plot_bottom - tt_h - scale_dpi(5, dpi);
                }

                let tt_rect = RECT {
                    left: tt_x,
                    top: tt_y,
                    right: tt_x + tt_w,
                    bottom: tt_y + tt_h,
                };

                let tt_brush = CreateSolidBrush(COLOR_TOOLTIP_BG);
                FillRect(hdc, &tt_rect, tt_brush);
                let _ = DeleteObject(tt_brush);

                let tt_border_pen = CreatePen(PS_SOLID, 1, COLOR_TOOLTIP_BORDER);
                let _ = SelectObject(hdc, tt_border_pen);
                let tt_box_pts = [
                    POINT { x: tt_x, y: tt_y },
                    POINT { x: tt_x + tt_w, y: tt_y },
                    POINT { x: tt_x + tt_w, y: tt_y + tt_h },
                    POINT { x: tt_x, y: tt_y + tt_h },
                    POINT { x: tt_x, y: tt_y },
                ];
                let _ = Polyline(hdc, &tt_box_pts);
                let _ = DeleteObject(tt_border_pen);

                let pad = scale_dpi(10, dpi);
                let line_h = scale_dpi(23, dpi);
                draw_text_line(hdc, tt_x + pad, tt_y + pad, &time_str, COLOR_TEXT_LIGHT);
                draw_text_line(hdc, tt_x + pad, tt_y + pad + line_h, &line_cpu, COLOR_CPU);
                draw_text_line(hdc, tt_x + pad, tt_y + pad + line_h * 2, &line_gpu, COLOR_GPU);

                let _ = DeleteObject(tooltip_font);
            }
        }
    }

    // Dọn dẹp tài nguyên GDI chặt chẽ (Zero GDI Leaks)
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(grid_pen);

    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(tab_font);
}

/// Window Procedure cho cửa sổ biểu đồ
unsafe extern "system" fn chart_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            // Chống giật nhấp nháy bằng cách thông báo đã xóa nền
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc_screen = BeginPaint(hwnd, &mut ps);

            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let width = client_rect.right - client_rect.left;
            let height = client_rect.bottom - client_rect.top;

            if width > 0 && height > 0 {
                let hdc_mem = CreateCompatibleDC(hdc_screen);
                let hbm_mem = CreateCompatibleBitmap(hdc_screen, width, height);
                let old_bm = SelectObject(hdc_mem, hbm_mem);

                let dpi = get_window_dpi(hwnd);
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ChartWindowState;
                if let Some(state) = ptr.as_ref() {
                    paint_chart(hdc_mem, width, height, state, dpi);
                }

                let _ = BitBlt(hdc_screen, 0, 0, width, height, hdc_mem, 0, 0, SRCCOPY);

                let _ = SelectObject(hdc_mem, old_bm);
                let _ = DeleteObject(hbm_mem);
                let _ = DeleteDC(hdc_mem);
            }

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

            let dpi = get_window_dpi(hwnd);
            if let Some(new_mode) = hit_test_tab_dpi(x, y, dpi) {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ChartWindowState;
                if let Some(state) = ptr.as_mut() {
                    if state.active_mode != new_mode {
                        state.active_mode = new_mode;
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u32;
            let target_mode = match key {
                0x31 | 0x61 => Some(ChartMode::TempAndFan), // Phím '1' hoặc NumPad1
                0x32 | 0x62 => Some(ChartMode::Power),      // Phím '2' hoặc NumPad2
                0x33 | 0x63 => Some(ChartMode::Clock),      // Phím '3' hoặc NumPad3
                _ => None,
            };

            if let Some(new_mode) = target_mode {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ChartWindowState;
                if let Some(state) = ptr.as_mut() {
                    if state.active_mode != new_mode {
                        state.active_mode = new_mode;
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_REFRESH {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ChartWindowState;
            if let Some(state) = ptr.as_mut() {
                state.mouse_pos = Some(POINT { x, y });
                if !state.is_mouse_tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    state.is_mouse_tracking = true;
                }
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ChartWindowState;
            if let Some(state) = ptr.as_mut() {
                state.mouse_pos = None;
                state.is_mouse_tracking = false;
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            // Giới hạn kích thước tối thiểu khi người dùng co dãn cửa sổ (thích ứng DPI)
            let dpi = get_window_dpi(hwnd);
            let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
            mmi.ptMinTrackSize.x = scale_dpi(600, dpi);
            mmi.ptMinTrackSize.y = scale_dpi(400, dpi);
            LRESULT(0)
        }
        WM_CLOSE => {
            // Dừng timer và ẩn cửa sổ khi đóng, giữ background thread và không tắt app
            let _ = KillTimer(hwnd, TIMER_REFRESH);
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(hwnd, TIMER_REFRESH);
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ChartWindowState;
            if !ptr.is_null() {
                let _ = Box::from_raw(ptr);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            ACTIVE_CHART_HWND.store(0, Ordering::Relaxed);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ============================================================================
// UNIT TESTS KIỂM CHỨNG TỌA ĐỘ VÀ CHUYỂN ĐỔI CHẾ ĐỘ
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_helper() {
        // #121417 -> R=18, G=20, B=23 -> 0x00171412
        assert_eq!(rgb(18, 20, 23), COLORREF(0x00171412));
        // #00E5FF -> R=0, G=229, B=255 -> 0x00FFE500
        assert_eq!(rgb(0, 229, 255), COLORREF(0x00FFE500));
        // #FF9100 -> R=255, G=145, B=0 -> 0x000091FF
        assert_eq!(rgb(255, 145, 0), COLORREF(0x000091FF));
        // #E53935 -> R=229, G=57, B=53 -> 0x003539E5
        assert_eq!(rgb(229, 57, 53), COLORREF(0x003539E5));
    }

    #[test]
    fn test_chart_mode_enum_and_conversion() {
        assert_eq!(ChartMode::default(), ChartMode::TempAndFan);
        assert_eq!(ChartMode::from_index(0), Some(ChartMode::TempAndFan));
        assert_eq!(ChartMode::from_index(1), Some(ChartMode::Power));
        assert_eq!(ChartMode::from_index(2), Some(ChartMode::Clock));
        assert_eq!(ChartMode::from_index(3), None);

        assert_eq!(ChartMode::TempAndFan.to_index(), 0);
        assert_eq!(ChartMode::Power.to_index(), 1);
        assert_eq!(ChartMode::Clock.to_index(), 2);

        assert_eq!(ChartMode::TempAndFan.label(), "🌡️ Nhiệt độ & Quạt");
        assert_eq!(ChartMode::Power.label(), "⚡ Công suất (W)");
        assert_eq!(ChartMode::Clock.label(), "⏱️ Xung nhịp (GHz)");
    }

    #[test]
    fn test_tab_hit_testing() {
        // Tab 0: x: 20..=180, y: 8..=34
        assert_eq!(hit_test_tab(20, 8), Some(ChartMode::TempAndFan));
        assert_eq!(hit_test_tab(100, 20), Some(ChartMode::TempAndFan));
        assert_eq!(hit_test_tab(180, 34), Some(ChartMode::TempAndFan));

        // Khoảng trống giữa tab 0 và 1 (x: 181..185)
        assert_eq!(hit_test_tab(183, 20), None);

        // Tab 1: x: 186..=326, y: 8..=34
        assert_eq!(hit_test_tab(186, 8), Some(ChartMode::Power));
        assert_eq!(hit_test_tab(250, 20), Some(ChartMode::Power));
        assert_eq!(hit_test_tab(326, 34), Some(ChartMode::Power));

        // Khoảng trống giữa tab 1 và 2 (x: 327..331)
        assert_eq!(hit_test_tab(329, 20), None);

        // Tab 2: x: 332..=482, y: 8..=34
        assert_eq!(hit_test_tab(332, 8), Some(ChartMode::Clock));
        assert_eq!(hit_test_tab(400, 20), Some(ChartMode::Clock));
        assert_eq!(hit_test_tab(482, 34), Some(ChartMode::Clock));

        // Ngoài vùng Y
        assert_eq!(hit_test_tab(100, 5), None);
        assert_eq!(hit_test_tab(100, 35), None);

        // Ngoài vùng X
        assert_eq!(hit_test_tab(10, 20), None);
        assert_eq!(hit_test_tab(500, 20), None);
    }

    #[test]
    fn test_time_x_coordinate_mapping() {
        let plot_left = 60;
        let plot_right = 775;
        let plot_width = plot_right - plot_left; // 715
        let newest_ts = 10000u64;

        // Mẫu mới nhất k = 0
        let x_0 = map_time_x(newest_ts, newest_ts, plot_left, plot_right);
        assert_eq!(x_0, Some(plot_right));

        // Mẫu cách 1 giờ k = 3600
        let x_3600 = map_time_x(newest_ts - 3600, newest_ts, plot_left, plot_right);
        assert_eq!(x_3600, Some(plot_left));

        // Mẫu cách 30 phút k = 1800 -> chính giữa
        let x_1800 = map_time_x(newest_ts - 1800, newest_ts, plot_left, plot_right).unwrap();
        let expected_mid = plot_left + (plot_width / 2);
        assert!((x_1800 - expected_mid).abs() <= 1);

        // Mẫu cũ hơn 1 giờ (>3600s) -> None
        let x_old = map_time_x(newest_ts - 3601, newest_ts, plot_left, plot_right);
        assert_eq!(x_old, None);
    }

    #[test]
    fn test_temp_y_coordinate_mapping() {
        let plot_top = 72;
        let plot_bottom = 480;

        // 30°C -> ở đáy plot (plot_bottom)
        assert_eq!(map_temp_y(30, plot_top, plot_bottom), plot_bottom);

        // 100°C -> ở đỉnh plot (plot_top)
        assert_eq!(map_temp_y(100, plot_top, plot_bottom), plot_top);

        // 65°C -> chính giữa trục Y
        assert_eq!(map_temp_y(65, plot_top, plot_bottom), (plot_bottom + plot_top) / 2);

        // Dưới 30°C -> clamp về đáy
        assert_eq!(map_temp_y(20, plot_top, plot_bottom), plot_bottom);

        // Trên 100°C -> clamp về đỉnh
        assert_eq!(map_temp_y(115, plot_top, plot_bottom), plot_top);
    }

    #[test]
    fn test_power_y_coordinate_mapping_and_scale() {
        let plot_top = 72;
        let plot_bottom = 480;

        // Thang đo 150W tiêu chuẩn
        let scale_150 = 150.0f32;
        assert_eq!(map_power_y(0.0, scale_150, plot_top, plot_bottom), plot_bottom);
        assert_eq!(map_power_y(150.0, scale_150, plot_top, plot_bottom), plot_top);
        assert_eq!(map_power_y(75.0, scale_150, plot_top, plot_bottom), (plot_bottom + plot_top) / 2);

        // Clamp
        assert_eq!(map_power_y(-10.0, scale_150, plot_top, plot_bottom), plot_bottom);
        assert_eq!(map_power_y(180.0, scale_150, plot_top, plot_bottom), plot_top);

        // Thang đo 200W co giãn
        let scale_200 = 200.0f32;
        assert_eq!(map_power_y(0.0, scale_200, plot_top, plot_bottom), plot_bottom);
        assert_eq!(map_power_y(200.0, scale_200, plot_top, plot_bottom), plot_top);
        assert_eq!(map_power_y(100.0, scale_200, plot_top, plot_bottom), (plot_bottom + plot_top) / 2);

        // Kiểm tra hàm tính scale tự động
        let sample_normal = [TelemetrySample {
            cpu_power_dw: 850, // 85.0 W
            gpu_power_dw: 1150, // 115.0 W
            ..Default::default()
        }];
        assert_eq!(calculate_power_scale(&sample_normal), 150.0);

        let sample_heavy = [TelemetrySample {
            cpu_power_dw: 1550, // 155.0 W (vượt 150W)
            gpu_power_dw: 1200, // 120.0 W
            ..Default::default()
        }];
        assert_eq!(calculate_power_scale(&sample_heavy), 200.0);
    }

    #[test]
    fn test_clock_y_coordinate_mapping() {
        let plot_top = 72;
        let plot_bottom = 480;

        // 0.0 GHz -> đáy
        assert_eq!(map_clock_y(0.0, plot_top, plot_bottom), plot_bottom);

        // 5.5 GHz -> đỉnh
        assert_eq!(map_clock_y(5.5, plot_top, plot_bottom), plot_top);

        // 2.75 GHz -> chính giữa
        assert_eq!(map_clock_y(2.75, plot_top, plot_bottom), (plot_bottom + plot_top) / 2);

        // Clamp
        assert_eq!(map_clock_y(-0.5, plot_top, plot_bottom), plot_bottom);
        assert_eq!(map_clock_y(6.0, plot_top, plot_bottom), plot_top);
    }

    #[test]
    fn test_chart_window_state_telemetry_interaction() {
        let telemetry = Arc::new(Mutex::new(TelemetryHistory::new()));
        let state = ChartWindowState {
            telemetry: telemetry.clone(),
            mouse_pos: None,
            is_mouse_tracking: false,
            active_mode: ChartMode::TempAndFan,
        };

        // Khi rỗng
        {
            let hist = state.telemetry.lock().unwrap();
            assert!(hist.is_empty());
            assert_eq!(hist.stats(), None);
        }

        // Sau khi đẩy dữ liệu với đầy đủ thông số (nhiệt độ, quạt, công suất, xung nhịp)
        {
            let mut hist = telemetry.lock().unwrap();
            hist.push_with_timestamp(100, 48, 42, 35, 30, 420, 190, 3800, 2200);
            hist.push_with_timestamp(101, 52, 45, 40, 35, 450, 210, 4200, 2300);
        }

        {
            let hist = state.telemetry.lock().unwrap();
            assert_eq!(hist.len(), 2);
            let stats = hist.stats().unwrap();
            assert_eq!(stats.cpu_current, 52);
            assert_eq!(stats.cpu_min, 48);
            assert_eq!(stats.cpu_max, 52);
            assert_eq!(stats.gpu_current, 45);
            assert_eq!(stats.cpu_power_current, 450);
            assert_eq!(stats.gpu_power_current, 210);
            assert_eq!(stats.cpu_mhz_current, 4200);
            assert_eq!(stats.gpu_mhz_current, 2300);
        }
    }

    #[test]
    fn test_dpi_scaling_helpers() {
        // 96 DPI (100% scale)
        assert_eq!(scale_dpi(96, 96), 96);
        assert_eq!(scale_dpi(36, 96), 36);
        assert_eq!(scale_dpi(28, 96), 28);

        // 120 DPI (125% scale)
        assert_eq!(scale_dpi(96, 120), 120);
        assert_eq!(scale_dpi(36, 120), 45);
        assert_eq!(scale_dpi(28, 120), 35);

        // 144 DPI (150% scale)
        assert_eq!(scale_dpi(96, 144), 144);
        assert_eq!(scale_dpi(36, 144), 54);

        // Font Em-Height: -MulDiv(pt, dpi, 72)
        // 13 pt tại 96 DPI: -(13 * 96 / 72) = -17
        assert_eq!(font_height_for_pt(13.0, 96), -17);
        // 13 pt tại 120 DPI: -(13 * 120 / 72) = -22
        assert_eq!(font_height_for_pt(13.0, 120), -22);
        // 12.5 pt tại 120 DPI: -(12.5 * 120 / 72) = -21
        assert_eq!(font_height_for_pt(12.5, 120), -21);
        // 11.5 pt tại 120 DPI: -(11.5 * 120 / 72) = -19
        assert_eq!(font_height_for_pt(11.5, 120), -19);
        // 11.0 pt tại 120 DPI: -(11.0 * 120 / 72) = -18
        assert_eq!(font_height_for_pt(11.0, 120), -18);
    }

    #[test]
    fn test_tab_rect_and_hit_testing_at_120_dpi() {
        let r0 = get_tab_rect_dpi(ChartMode::TempAndFan, 120);
        let r1 = get_tab_rect_dpi(ChartMode::Power, 120);
        let r2 = get_tab_rect_dpi(ChartMode::Clock, 120);

        // Chiều cao tab tại 120 DPI (36 * 120 / 96 = 45 px)
        assert_eq!(r0.bottom - r0.top, 45);
        assert_eq!(r1.bottom - r1.top, 45);
        assert_eq!(r2.bottom - r2.top, 45);

        // Hit-test tại 120 DPI
        assert_eq!(hit_test_tab_dpi(r0.left + 5, r0.top + 5, 120), Some(ChartMode::TempAndFan));
        assert_eq!(hit_test_tab_dpi(r1.left + 5, r1.top + 5, 120), Some(ChartMode::Power));
        assert_eq!(hit_test_tab_dpi(r2.left + 5, r2.top + 5, 120), Some(ChartMode::Clock));

        // Ngoài vùng tab tại 120 DPI
        assert_eq!(hit_test_tab_dpi(0, 0, 120), None);
    }
}
