//! Acer Predator Fan Controller - Dashboard & Hardware Control Center
//!
//! Giao diện bảng điều khiển trung tâm (Win32 GDI thuần, Event-Driven, Double-Buffering):
//! - Bố cục chuẩn 540 x 450 px, căn giữa màn hình, cố định kích thước chống vỡ layout.
//! - Hệ thống Theme: Predator Dark (#121417) và Clean Light (#F4F6F9) chuyển đổi tức thì.
//! - Nút bấm phản hồi xúc giác (Tactile Buttons) bo góc mượt mà (RoundRect):
//!   + Hover: nền sáng hơn, viền sáng màu accent.
//!   + Pressed: lún xuống 2px và dịch chuyển sang phải 1px.
//!   + Active: nền sáng màu nhấn, viền accent 2px.
//! - 2 Card đo đạc CPU & GPU Telemetry: Nhiệt độ to 32pt Segoe UI Bold, thanh % tiến trình quạt, nhãn RPM.
//! - Bộ 3 nút chọn Profile quạt: [ Active Curve ] | [ Turbo Max ] | [ Silent ].
//! - Thanh điều khiển nhiệt độ CPU (Smooth EPP Governor) kèm nút Toggle BẬT/TẮT.
//! - Lưới 4 nút công cụ chức năng: Tự chỉnh Curve, Biểu đồ 1 Giờ, Tải lại Config, Thoát ứng dụng.
//! - Double-Buffering hoàn toàn trên Memory DC, chống nhấp nháy (zero-flicker).
//! - Quản lý GDI chặt chẽ: 100% GDI objects được giải phóng, không rò rỉ RAM (0 GDI leak).
//! - Quản lý vòng đời Singleton: Thu nhỏ hoặc bấm [X] chỉ ẩn xuống khay, dừng timer để CPU về 0.00%.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use windows::core::{w, PCWSTR, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE};
use windows::Win32::UI::WindowsAndMessaging::*;

#[link(name = "kernel32")]
extern "system" {
    fn MulDiv(n_number: i32, n_numerator: i32, n_denominator: i32) -> i32;
}

use crate::telemetry::TelemetryHistory;
use crate::tray::{FanProfile, TrayCommand, TrayState};

/// Handle cửa sổ Dashboard đang mở (Singleton)
pub static ACTIVE_DASHBOARD_HWND: AtomicIsize = AtomicIsize::new(0);

const IDT_DASHBOARD_REFRESH: usize = 1;
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Helper macro/hàm tạo COLORREF Win32: 0x00BBGGRR
#[inline]
pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

/// Màu Tím Neon Win32 COLORREF BGR: RGB(188, 140, 255) -> 0x00FF8CBC
pub const COLOR_SYS_TEMP: COLORREF = rgb(188, 140, 255);

/// Chế độ hiển thị Giao diện (Theme)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTheme {
    Dark,
    Light,
}

/// Bảng mã màu Win32 cho từng Theme
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePalette {
    pub bg: COLORREF,
    pub card: COLORREF,
    pub border: COLORREF,
    pub btn_norm: COLORREF,
    pub btn_hover: COLORREF,
    pub btn_pressed: COLORREF,
    pub cpu: COLORREF,
    pub gpu: COLORREF,
    pub text: COLORREF,
    pub text_sec: COLORREF,
    pub danger: COLORREF,
    pub active_bg: COLORREF,
    pub bar_bg: COLORREF,
}

impl AppTheme {
    pub fn palette(&self) -> ThemePalette {
        match self {
            AppTheme::Dark => ThemePalette {
                bg: rgb(18, 20, 23),          // #121417
                card: rgb(22, 25, 30),        // #16191E
                border: rgb(52, 59, 72),      // #343B48
                btn_norm: rgb(30, 34, 41),    // #1E2229
                btn_hover: rgb(40, 46, 56),   // #282E38
                btn_pressed: rgb(15, 17, 20), // #0F1114
                cpu: rgb(0, 240, 255),        // #00F0FF (Electric Cyan sáng rực)
                gpu: rgb(255, 167, 38),       // #FFA726 (Vàng cam sáng)
                text: rgb(248, 250, 252),     // #F8FAFC (Trắng ngọc siêu sáng rõ)
                text_sec: rgb(208, 217, 230), // #D0D9E6 (Sáng rõ nét, độ tương phản cực cao trên nền #16191E)
                danger: rgb(255, 59, 48),     // #FF3B30
                active_bg: rgb(0, 65, 80),    // Dark cyan tint
                bar_bg: rgb(15, 17, 20),      // Bar track #0F1114
            },
            AppTheme::Light => ThemePalette {
                bg: rgb(244, 246, 249),          // #F4F6F9
                card: rgb(255, 255, 255),        // #FFFFFF
                border: rgb(208, 215, 222),      // #D0D7DE
                btn_norm: rgb(234, 236, 239),    // #EAECEF
                btn_hover: rgb(220, 225, 231),   // #DCE1E7
                btn_pressed: rgb(200, 208, 216), // #C8D0D8
                cpu: rgb(0, 151, 167),           // #0097A7
                gpu: rgb(230, 81, 0),            // #E65100
                text: rgb(31, 35, 40),           // #1F2328
                text_sec: rgb(87, 96, 106),      // #57606A
                danger: rgb(211, 47, 47),        // #D32F2F
                active_bg: rgb(200, 235, 240),   // Light cyan tint
                bar_bg: rgb(220, 225, 230),      // Light track
            },
        }
    }
}

/// Định danh các nút tương tác trên Dashboard
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonId {
    ProfileCurve,
    ProfileTurbo,
    ProfileSilent,
    ToggleEpp,
    OpenCurveEditor,
    OpenTelemetryChart,
    ReloadConfig,
    ExitApp,
    ToggleTheme,
}

/// Trạng thái tương tác xúc giác của nút bấm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Hovered,
    Pressed,
}

/// Nút bấm xúc giác với hiệu ứng nảy/lún và bo góc RoundRect
#[derive(Debug, Clone)]
pub struct TactileButton {
    pub id: ButtonId,
    pub rect: RECT,
    pub label: String,
    pub state: ButtonState,
    pub is_active: bool,
    pub is_danger: bool,
}

/// Bản chụp trạng thái tức thời không khóa (Zero-Contention Snapshot)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DashboardSnapshot {
    pub cpu_temp: u32,
    pub cpu_speed: u8,
    pub gpu_temp: u32,
    pub gpu_speed: u8,
    pub cpu_rpm: u32,
    pub gpu_rpm: u32,
    pub cpu_power_dw: u16,
    pub gpu_power_dw: u16,
    pub cpu_mhz: u16,
    pub gpu_mhz: u16,
    pub system_temp: u32,
    pub active_profile: FanProfile,
    pub governor_enabled: bool,
    pub governor_target_temp: u32,
    pub governor_epp: u32,
    pub reload_flash_ticks: u8,
}

impl Default for DashboardSnapshot {
    fn default() -> Self {
        Self {
            cpu_temp: 0,
            cpu_speed: 0,
            gpu_temp: 0,
            gpu_speed: 0,
            cpu_rpm: 0,
            gpu_rpm: 0,
            cpu_power_dw: 0,
            gpu_power_dw: 0,
            cpu_mhz: 0,
            gpu_mhz: 0,
            system_temp: 0,
            active_profile: FanProfile::ActiveCurve,
            governor_enabled: true,
            governor_target_temp: 80,
            governor_epp: 0,
            reload_flash_ticks: 0,
        }
    }
}

impl DashboardSnapshot {
    pub fn from_state(state: &TrayState, reload_flash_ticks: u8) -> Self {
        let (cpu_temp, cpu_speed, gpu_temp, gpu_speed, cpu_rpm, gpu_rpm) = state.get_status_rpm();
        let (cpu_power_dw, gpu_power_dw, cpu_mhz, gpu_mhz) = state.get_metrics();
        let (governor_enabled, governor_target_temp, governor_epp) = state.get_governor_info();
        let active_profile = state.get_profile();
        let system_temp = state.system_temp.load(Ordering::Relaxed);

        Self {
            cpu_temp,
            cpu_speed,
            gpu_temp,
            gpu_speed,
            cpu_rpm,
            gpu_rpm,
            cpu_power_dw,
            gpu_power_dw,
            cpu_mhz,
            gpu_mhz,
            system_temp,
            active_profile,
            governor_enabled,
            governor_target_temp,
            governor_epp,
            reload_flash_ticks,
        }
    }
}

/// Trạng thái nội bộ của cửa sổ Dashboard gắn vào GWLP_USERDATA
struct DashboardWindowState {
    state: Arc<TrayState>,
    cmd_sender: Sender<TrayCommand>,
    telemetry: Arc<Mutex<TelemetryHistory>>,
    theme: AppTheme,
    hovered_btn: Option<ButtonId>,
    pressed_btn: Option<ButtonId>,
    is_mouse_tracking: bool,
    reload_flash_ticks: u8,
}

/// Kiểm tra tọa độ điểm có nằm trong hình chữ nhật hay không (Hit-testing an toàn)
#[inline]
pub fn rect_contains(rect: &RECT, pt: POINT) -> bool {
    pt.x >= rect.left && pt.x < rect.right && pt.y >= rect.top && pt.y < rect.bottom
}

/// Tìm kiếm ButtonId tương ứng với vị trí con trỏ chuột
pub fn find_button_at(buttons: &[TactileButton], pt: POINT) -> Option<ButtonId> {
    for btn in buttons {
        if rect_contains(&btn.rect, pt) {
            return Some(btn.id);
        }
    }
    None
}

/// Bố cục động hoàn chỉnh của Dashboard (Cards, Panels và Buttons)
#[derive(Debug, Clone)]
pub struct DashboardLayout {
    pub card_cpu_rect: RECT,
    pub card_gpu_rect: RECT,
    pub strip_system_rect: RECT,
    pub epp_panel_rect: RECT,
    pub buttons: Vec<TactileButton>,
}

/// Tính toán toàn bộ bố cục Dashboard đáp ứng linh hoạt theo kích thước cửa sổ (Responsive Layout)
pub fn compute_dashboard_layout(
    width: i32,
    height: i32,
    snapshot: &DashboardSnapshot,
    _theme: AppTheme,
    hovered: Option<ButtonId>,
    pressed: Option<ButtonId>,
) -> DashboardLayout {
    let w = width.max(480);
    let h = height.max(440);

    let get_state = |id: ButtonId| -> ButtonState {
        if pressed == Some(id) {
            ButtonState::Pressed
        } else if hovered == Some(id) {
            ButtonState::Hovered
        } else {
            ButtonState::Normal
        }
    };

    // 1. Nút đổi Giao diện (Header Theme Toggle): Chiều rộng 135 px, neo ở w - 145, top: 10, bottom: 40
    let theme_btn_rect = RECT {
        left: w - 145,
        top: 10,
        right: w - 10,
        bottom: 40,
    };

    // Phân bổ chiều dọc linh hoạt (Proportional Vertical Flex Layout - Không để lại khoảng trống đen)
    let content_top = 48;
    let content_bottom = h - 16;
    let available_content = (content_bottom - content_top).max(370);

    let prof_h = ((available_content * 9) / 100).clamp(38, 44);
    let epp_h = prof_h;
    let tool_row_h = prof_h;
    let strip_h = 34; // Chiều cao thanh SYSTEM / MAINBOARD Telemetry Strip

    let base_gap = ((available_content * 28) / 1000).clamp(10, 20);
    let tool_gap = (base_gap * 3 / 4).max(8);

    let total_fixed_lower = strip_h + prof_h + epp_h + (tool_row_h * 2) + (base_gap * 4) + tool_gap;
    let card_h = (available_content - total_fixed_lower).max(140);
    let card_bottom = content_top + card_h;

    // Dải SYSTEM / MAINBOARD Telemetry Strip nằm giữa 2 card và hàng profile buttons
    let strip_y_top = card_bottom + base_gap;
    let strip_y_bottom = strip_y_top + strip_h;
    let strip_system_rect = RECT {
        left: 16,
        top: strip_y_top,
        right: w - 16,
        bottom: strip_y_bottom,
    };

    let prof_y_top = strip_y_bottom + base_gap;
    let prof_y_bottom = prof_y_top + prof_h;

    let epp_y_top = prof_y_bottom + base_gap;
    let epp_y_bottom = epp_y_top + epp_h;

    let tools_y_top = epp_y_bottom + base_gap;
    let tool_r1_top = tools_y_top;
    let tool_r1_bottom = tool_r1_top + tool_row_h;
    let tool_r2_top = tool_r1_bottom + tool_gap;
    let tool_r2_bottom = tool_r2_top + tool_row_h;

    // --- Cards Rect ---
    let card_gap = 12;
    let card_w = (w - 32 - card_gap) / 2;
    let card_cpu_rect = RECT {
        left: 16,
        top: content_top,
        right: 16 + card_w,
        bottom: card_bottom,
    };
    let card_gpu_rect = RECT {
        left: 16 + card_w + card_gap,
        top: content_top,
        right: w - 16,
        bottom: card_bottom,
    };

    // --- Profiles Buttons ---
    let prof_gap = 8;
    let prof_width = (w - 32 - (prof_gap * 2)) / 3;
    let p1_rect = RECT {
        left: 16,
        top: prof_y_top,
        right: 16 + prof_width,
        bottom: prof_y_bottom,
    };
    let p2_rect = RECT {
        left: 16 + prof_width + prof_gap,
        top: prof_y_top,
        right: 16 + (prof_width * 2) + prof_gap,
        bottom: prof_y_bottom,
    };
    let p3_rect = RECT {
        left: 16 + (prof_width * 2) + (prof_gap * 2),
        top: prof_y_top,
        right: w - 16,
        bottom: prof_y_bottom,
    };

    // --- EPP Panel & Toggle Button ---
    let epp_panel_rect = RECT {
        left: 16,
        top: epp_y_top,
        right: w - 16,
        bottom: epp_y_bottom,
    };
    let epp_btn_w = 148;
    let epp_btn_h = 32.min(epp_h - 4);
    let epp_btn_top = epp_y_top + (epp_h - epp_btn_h) / 2;
    let epp_btn_right = w - 24;
    let epp_btn_left = epp_btn_right - epp_btn_w;
    let epp_btn_rect = RECT {
        left: epp_btn_left,
        top: epp_btn_top,
        right: epp_btn_right,
        bottom: epp_btn_top + epp_btn_h,
    };
    let epp_label = if snapshot.governor_enabled {
        format!("BẬT • {}°C", snapshot.governor_target_temp)
    } else {
        "TẮT".to_string()
    };

    // --- Tools Grid ---
    let half_w = w / 2;
    let grid_gap = 10;
    let col1_left = 16;
    let col1_right = half_w - (grid_gap / 2);
    let col2_left = half_w + (grid_gap / 2);
    let col2_right = w - 16;

    let mut buttons = Vec::with_capacity(9);

    buttons.push(TactileButton {
        id: ButtonId::ToggleTheme,
        rect: theme_btn_rect,
        label: "🌓 Giao diện".to_string(),
        state: get_state(ButtonId::ToggleTheme),
        is_active: false,
        is_danger: false,
    });

    buttons.push(TactileButton {
        id: ButtonId::ProfileCurve,
        rect: p1_rect,
        label: "⚡ Active Curve".to_string(),
        state: get_state(ButtonId::ProfileCurve),
        is_active: snapshot.active_profile == FanProfile::ActiveCurve,
        is_danger: false,
    });

    buttons.push(TactileButton {
        id: ButtonId::ProfileTurbo,
        rect: p2_rect,
        label: "🔥 Turbo Max".to_string(),
        state: get_state(ButtonId::ProfileTurbo),
        is_active: snapshot.active_profile == FanProfile::Turbo,
        is_danger: false,
    });

    buttons.push(TactileButton {
        id: ButtonId::ProfileSilent,
        rect: p3_rect,
        label: "🍃 Silent".to_string(),
        state: get_state(ButtonId::ProfileSilent),
        is_active: snapshot.active_profile == FanProfile::Silent,
        is_danger: false,
    });

    buttons.push(TactileButton {
        id: ButtonId::ToggleEpp,
        rect: epp_btn_rect,
        label: epp_label,
        state: get_state(ButtonId::ToggleEpp),
        is_active: snapshot.governor_enabled,
        is_danger: false,
    });

    buttons.push(TactileButton {
        id: ButtonId::OpenCurveEditor,
        rect: RECT {
            left: col1_left,
            top: tool_r1_top,
            right: col1_right,
            bottom: tool_r1_bottom,
        },
        label: "⚙ Tự chỉnh Curve".to_string(),
        state: get_state(ButtonId::OpenCurveEditor),
        is_active: false,
        is_danger: false,
    });

    buttons.push(TactileButton {
        id: ButtonId::OpenTelemetryChart,
        rect: RECT {
            left: col2_left,
            top: tool_r1_top,
            right: col2_right,
            bottom: tool_r1_bottom,
        },
        label: "📈 Biểu đồ 1 Giờ".to_string(),
        state: get_state(ButtonId::OpenTelemetryChart),
        is_active: false,
        is_danger: false,
    });

    buttons.push(TactileButton {
        id: ButtonId::ReloadConfig,
        rect: RECT {
            left: col1_left,
            top: tool_r2_top,
            right: col1_right,
            bottom: tool_r2_bottom,
        },
        label: "🔄 Tải lại Config".to_string(),
        state: get_state(ButtonId::ReloadConfig),
        is_active: false,
        is_danger: false,
    });

    buttons.push(TactileButton {
        id: ButtonId::ExitApp,
        rect: RECT {
            left: col2_left,
            top: tool_r2_top,
            right: col2_right,
            bottom: tool_r2_bottom,
        },
        label: "✕ Thoát ứng dụng".to_string(),
        state: get_state(ButtonId::ExitApp),
        is_active: false,
        is_danger: true,
    });

    DashboardLayout {
        card_cpu_rect,
        card_gpu_rect,
        strip_system_rect,
        epp_panel_rect,
        buttons,
    }
}

/// Tính toán bố cục các nút bấm dựa theo kích thước Client Area
pub fn compute_button_layout(
    width: i32,
    height: i32,
    snapshot: &DashboardSnapshot,
    theme: AppTheme,
    hovered: Option<ButtonId>,
    pressed: Option<ButtonId>,
) -> Vec<TactileButton> {
    compute_dashboard_layout(width, height, snapshot, theme, hovered, pressed).buttons
}

/// Mở hoặc đưa cửa sổ Dashboard lên phía trước (Singleton Handle)
pub fn show_or_focus_dashboard(
    state: Arc<crate::tray::TrayState>,
    cmd_sender: std::sync::mpsc::Sender<crate::tray::TrayCommand>,
    telemetry: Arc<std::sync::Mutex<crate::telemetry::TelemetryHistory>>,
) {
    let existing = ACTIVE_DASHBOARD_HWND.load(Ordering::Relaxed);
    if existing != 0 {
        let hwnd = HWND(existing as *mut _);
        unsafe {
            if IsWindow(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
                let _ = BringWindowToTop(hwnd);
                let _ = SetTimer(hwnd, IDT_DASHBOARD_REFRESH, 1000, None);
                let _ = InvalidateRect(hwnd, None, false);
                return;
            }
        }
    }

    thread::spawn(move || {
        if let Err(e) = run_dashboard_window(state, cmd_sender, telemetry) {
            eprintln!("[DashboardWindow] Lỗi khởi tạo cửa sổ bảng điều khiển: {:?}", e);
        }
    });
}

/// Chạy vòng lặp thông điệp của cửa sổ Dashboard trên thread riêng
fn run_dashboard_window(
    state: Arc<crate::tray::TrayState>,
    cmd_sender: Sender<TrayCommand>,
    telemetry: Arc<Mutex<TelemetryHistory>>,
) -> Result<()> {
    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null())?;
        let class_name = w!("PredatorDashboardWndClass");
        let app_icon = crate::tray::load_embedded_app_icon();

        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(dashboard_wndproc),
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

        // Kích thước chuẩn cơ sở: 600 x 548 px, nhân theo tỉ lệ DPI thực tế (tăng 48px cho Telemetry Strip)
        let sys_dpi = GetDpiForSystem();
        let dpi = if sys_dpi == 0 { 96 } else { sys_dpi as i32 };
        let width = MulDiv(600, dpi, 96);
        let height = MulDiv(548, dpi, 96);
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let pos_x = (screen_w - width) / 2;
        let pos_y = (screen_h - height) / 2;

        let window_style = WS_OVERLAPPEDWINDOW;

        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW | WS_EX_TOPMOST,
            class_name,
            w!("Predator Fan Control // Dashboard"),
            window_style | WS_VISIBLE,
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

        ACTIVE_DASHBOARD_HWND.store(hwnd.0 as isize, Ordering::Relaxed);

        let win_state = Box::new(DashboardWindowState {
            state,
            cmd_sender,
            telemetry,
            theme: AppTheme::Dark,
            hovered_btn: None,
            pressed_btn: None,
            is_mouse_tracking: false,
            reload_flash_ticks: 0,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(win_state) as isize);

        let _ = SetTimer(hwnd, IDT_DASHBOARD_REFRESH, 1000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        ACTIVE_DASHBOARD_HWND.store(0, Ordering::Relaxed);
        Ok(())
    }
}

/// Chuẩn hóa hàm tạo Font DPI-Aware
unsafe fn create_dpi_font(hwnd: HWND, point_size: i32, weight: i32) -> HFONT {
    let dpi = GetDpiForWindow(hwnd);
    let dpi = if dpi == 0 {
        let s = GetDpiForSystem();
        if s == 0 { 96 } else { s as i32 }
    } else {
        dpi as i32
    };
    let height = -MulDiv(point_size, dpi, 72);
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

/// Tạo Font DPI-Aware với kích thước lẻ (1/10 point)
unsafe fn create_dpi_font_tenth(hwnd: HWND, point_size_x10: i32, weight: i32) -> HFONT {
    let dpi = GetDpiForWindow(hwnd);
    let dpi = if dpi == 0 {
        let s = GetDpiForSystem();
        if s == 0 { 96 } else { s as i32 }
    } else {
        dpi as i32
    };
    let height = -MulDiv(point_size_x10, dpi, 720);
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

/// Helper vẽ chuỗi văn bản với DrawTextW và clipping an toàn
unsafe fn draw_text_clipped(
    hdc: HDC,
    mut rect: RECT,
    text: &str,
    color: COLORREF,
    flags: DRAW_TEXT_FORMAT,
) {
    SetTextColor(hdc, color);
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let _ = DrawTextW(hdc, &mut wide, &mut rect, flags);
}

/// Vẽ một Card Telemetry (CPU hoặc GPU) theo kiến trúc Multi-Line với Clipping Bounds an toàn
unsafe fn draw_telemetry_card(
    hdc: HDC,
    card_rect: RECT,
    title: &str,
    temp: u32,
    power_dw: u16,
    mhz: u16,
    speed: u8,
    rpm: u32,
    accent_color: COLORREF,
    palette: &ThemePalette,
    font_header: HFONT,
    font_temp: HFONT,
    font_metric: HFONT,
) {
    let card_pen = CreatePen(PS_SOLID, 1, palette.border);
    let card_brush = CreateSolidBrush(palette.card);
    let old_pen = SelectObject(hdc, card_pen);
    let old_brush = SelectObject(hdc, card_brush);

    let _ = RoundRect(
        hdc,
        card_rect.left,
        card_rect.top,
        card_rect.right,
        card_rect.bottom,
        10,
        10,
    );

    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(card_brush);
    let _ = DeleteObject(card_pen);

    // Bounding Box lề trong an toàn: lùi 12px mỗi bên, tuyệt đối không tràn ra ngoài viền card
    let inner_left = card_rect.left + 12;
    let inner_right = card_rect.right - 12;
    let card_h = card_rect.bottom - card_rect.top;

    let pad_top = (card_h * 7 / 100).clamp(8, 16);
    let line_h = 24;
    let rem_space = card_h - 146 - (pad_top * 2);
    let base_item_gap = (rem_space / 5).clamp(3, 16);

    let y1 = card_rect.top + pad_top;
    let rect_header = RECT { left: inner_left, top: y1, right: inner_right, bottom: y1 + line_h };

    let y2 = rect_header.bottom + base_item_gap.min(6);
    let rect_temp = RECT { left: inner_left, top: y2, right: inner_right, bottom: y2 + 40 };

    let y3 = rect_temp.bottom + base_item_gap;
    let rect_pwr = RECT { left: inner_left, top: y3, right: inner_right, bottom: y3 + line_h };

    let y4 = rect_pwr.bottom + (base_item_gap / 2).max(2);
    let rect_clk = RECT { left: inner_left, top: y4, right: inner_right, bottom: y4 + line_h };

    let y5 = rect_clk.bottom + base_item_gap;
    let rect_bar = RECT { left: inner_left, top: y5, right: inner_right, bottom: y5 + 10 };

    let y6 = rect_bar.bottom + (base_item_gap / 2).max(4);
    let rect_fan = RECT { left: inner_left, top: y6, right: inner_right, bottom: y6 + line_h };

    // Dòng 1: Header (Font 11.5pt Bold, màu accent)
    let _ = SelectObject(hdc, font_header);
    draw_text_clipped(
        hdc,
        rect_header,
        title,
        accent_color,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    // Dòng 2: Nhiệt độ (Font Segoe UI 28pt Bold, màu accent)
    let _ = SelectObject(hdc, font_temp);
    let temp_str = format!("{}°C", temp);
    draw_text_clipped(
        hdc,
        rect_temp,
        &temp_str,
        accent_color,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    // Dòng 3: Công suất (Font 10.5pt Semi-Bold, màu palette.text_sec)
    let _ = SelectObject(hdc, font_metric);
    let pwr = power_dw as f32 / 10.0;
    let pwr_str = format!("⚡ Công suất: {:.1} W", pwr);
    draw_text_clipped(
        hdc,
        rect_pwr,
        &pwr_str,
        palette.text_sec,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    // Dòng 4: Xung nhịp (Font 10.5pt Semi-Bold, màu palette.text_sec)
    let ghz = mhz as f32 / 1000.0;
    let clk_str = format!("⏱ Xung nhịp: {:.2} GHz", ghz);
    draw_text_clipped(
        hdc,
        rect_clk,
        &clk_str,
        palette.text_sec,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    // Dòng 5: Thanh Bar quạt (chiều cao 9px, nền palette.bar_bg, thanh tiến trình accent)
    let track_brush = CreateSolidBrush(palette.bar_bg);
    FillRect(hdc, &rect_bar, track_brush);
    let _ = DeleteObject(track_brush);

    let track_w = rect_bar.right - rect_bar.left;
    let fill_w = (track_w * (speed.min(100) as i32)) / 100;
    if fill_w > 0 {
        let fill_rect = RECT {
            left: rect_bar.left,
            top: rect_bar.top,
            right: rect_bar.left + fill_w,
            bottom: rect_bar.bottom,
        };
        let fill_brush = CreateSolidBrush(accent_color);
        FillRect(hdc, &fill_rect, fill_brush);
        let _ = DeleteObject(fill_brush);
    }

    // Dòng 6: Thông số quạt (Font 10.5pt Semi-Bold, màu palette.text_sec)
    let fan_str = if rpm > 0 {
        format!("🌀 Quạt: {}% ({} RPM)", speed, rpm)
    } else {
        format!("🌀 Quạt: {}%", speed)
    };
    draw_text_clipped(
        hdc,
        rect_fan,
        &fan_str,
        palette.text_sec,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
}

/// Vẽ dải thông số nhiệt độ Mainboard / System (System Telemetry Strip - Lựa chọn B nâng cấp)
unsafe fn draw_system_telemetry_strip(
    hdc: HDC,
    strip_rect: RECT,
    system_temp: u32,
    palette: &ThemePalette,
    font_label: HFONT,
    font_temp: HFONT,
) {
    // 1. Khung nền glassmorphic bo góc 10px đồng bộ, viền mảnh palette.border
    let strip_pen = CreatePen(PS_SOLID, 1, palette.border);
    let strip_brush = CreateSolidBrush(palette.card);
    let old_pen = SelectObject(hdc, strip_pen);
    let old_brush = SelectObject(hdc, strip_brush);

    let _ = RoundRect(
        hdc,
        strip_rect.left,
        strip_rect.top,
        strip_rect.right,
        strip_rect.bottom,
        10,
        10,
    );

    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(strip_brush);
    let _ = DeleteObject(strip_pen);

    let strip_h = strip_rect.bottom - strip_rect.top;
    let cy = strip_rect.top + strip_h / 2;

    // 2. Bên trái: Icon / biểu tượng bo tròn nhỏ + Nhãn "MAINBOARD / SYSTEM"
    let dot_radius = 4;
    let dot_cx = strip_rect.left + 16;
    let dot_color = if system_temp > 0 {
        COLOR_SYS_TEMP
    } else {
        palette.border
    };
    let dot_brush = CreateSolidBrush(dot_color);
    let dot_pen = CreatePen(PS_SOLID, 1, dot_color);
    let old_pen = SelectObject(hdc, dot_pen);
    let old_brush = SelectObject(hdc, dot_brush);

    let _ = Ellipse(
        hdc,
        dot_cx - dot_radius,
        cy - dot_radius,
        dot_cx + dot_radius + 1,
        cy + dot_radius + 1,
    );

    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(dot_brush);
    let _ = DeleteObject(dot_pen);

    // Nhãn "MAINBOARD / SYSTEM" (màu text mờ palette.text_sec)
    let label_rect = RECT {
        left: dot_cx + dot_radius + 10,
        top: strip_rect.top,
        right: strip_rect.left + 240,
        bottom: strip_rect.bottom,
    };
    let _ = SelectObject(hdc, font_label);
    draw_text_clipped(
        hdc,
        label_rect,
        "MAINBOARD / SYSTEM",
        palette.text_sec,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    // 3. Bên phải: Chữ số nhiệt độ (Tím Neon #BC8CFF) + Mini Level Bar (30°C - 100°C)
    let (temp_str, temp_color) = if system_temp > 0 {
        (format!("{}°C", system_temp), COLOR_SYS_TEMP)
    } else {
        ("-- °C".to_string(), rgb(128, 138, 150)) // Xám mờ khi 0
    };

    let _ = SelectObject(hdc, font_temp);
    let wide_temp: Vec<u16> = temp_str.encode_utf16().collect();
    let mut size_temp = windows::Win32::Foundation::SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide_temp, &mut size_temp);

    let temp_right = strip_rect.right - 16;
    let temp_left = temp_right - size_temp.cx;
    let temp_rect = RECT {
        left: temp_left,
        top: strip_rect.top,
        right: temp_right,
        bottom: strip_rect.bottom,
    };
    draw_text_clipped(
        hdc,
        temp_rect,
        &temp_str,
        temp_color,
        DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    // Mini level bar: nằm bên trái temp_rect
    // Chiều rộng bar: 70px, chiều cao: 6px
    let bar_w = 70;
    let bar_h = 6;
    let bar_right = temp_left - 12;
    let bar_left = bar_right - bar_w;
    let bar_top = cy - (bar_h / 2);
    let bar_bottom = bar_top + bar_h;

    let bar_track_rect = RECT {
        left: bar_left,
        top: bar_top,
        right: bar_right,
        bottom: bar_bottom,
    };

    // Nền thanh bar (palette.bar_bg)
    let track_brush = CreateSolidBrush(palette.bar_bg);
    FillRect(hdc, &bar_track_rect, track_brush);
    let _ = DeleteObject(track_brush);

    // Tiến trình nhiệt độ (30°C ..= 100°C)
    if system_temp >= 30 {
        let clamped = system_temp.clamp(30, 100);
        let ratio = (clamped - 30) as f32 / 70.0;
        let fill_w = ((bar_w as f32) * ratio).round() as i32;
        if fill_w > 0 {
            let fill_rect = RECT {
                left: bar_left,
                top: bar_top,
                right: (bar_left + fill_w).min(bar_right),
                bottom: bar_bottom,
            };
            let fill_brush = CreateSolidBrush(COLOR_SYS_TEMP);
            FillRect(hdc, &fill_rect, fill_brush);
            let _ = DeleteObject(fill_brush);
        }
    }
}

/// Vẽ cụm công tắc gạt xúc giác (Modern Tactile Pill Toggle Switch)
/// Gồm nhãn trạng thái bên trái và thanh ray viên thuốc kèm bi gạt bên phải
unsafe fn draw_tactile_toggle_switch(
    hdc: HDC,
    btn: &TactileButton,
    palette: &ThemePalette,
    font_normal: HFONT,
    font_bold: HFONT,
) {
    let mut draw_rect = btn.rect;

    // Hiệu ứng lún xúc giác nhẹ khi click (Pressed): dịch chuyển +1px ngang, +1px dọc
    if btn.state == ButtonState::Pressed {
        draw_rect.left += 1;
        draw_rect.right += 1;
        draw_rect.top += 1;
        draw_rect.bottom += 1;
    }

    // 1. Kích thước & Tọa độ thanh ray (Pill Track): 50px width x 24px height, bo góc 24px
    let track_w = 50;
    let track_h = 24;
    let track_right = draw_rect.right - 4;
    let track_left = track_right - track_w;
    let track_top = draw_rect.top + (draw_rect.bottom - draw_rect.top - track_h) / 2;
    let track_bottom = track_top + track_h;
    let track_rect = RECT {
        left: track_left,
        top: track_top,
        right: track_right,
        bottom: track_bottom,
    };

    // 2. Màu sắc thanh ray theo trạng thái BẬT/TẮT và Hover/Pressed
    let (track_bg, track_border, border_width) = if btn.is_active {
        match btn.state {
            ButtonState::Hovered => (palette.cpu, rgb(180, 248, 255), 2),
            ButtonState::Pressed => (palette.active_bg, palette.cpu, 2),
            ButtonState::Normal => (palette.cpu, rgb(128, 242, 255), 1),
        }
    } else {
        match btn.state {
            ButtonState::Hovered => (palette.btn_hover, rgb(88, 96, 105), 1),
            ButtonState::Pressed => (palette.btn_pressed, palette.border, 1),
            ButtonState::Normal => (palette.btn_norm, palette.border, 1),
        }
    };

    let track_pen = CreatePen(PS_SOLID, border_width, track_border);
    let track_brush = CreateSolidBrush(track_bg);
    let old_pen = SelectObject(hdc, track_pen);
    let old_brush = SelectObject(hdc, track_brush);

    // Bo góc 24px với height 24px tạo thành hình viên thuốc (Pill shape) hoàn hảo
    let _ = RoundRect(
        hdc,
        track_rect.left,
        track_rect.top,
        track_rect.right,
        track_rect.bottom,
        24,
        24,
    );

    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(track_brush);
    let _ = DeleteObject(track_pen);

    // 3. Bi gạt (Thumb / Knob): đường kính 18px, lề trong 3px
    let thumb_top = track_rect.top + 3;
    let thumb_bottom = thumb_top + 18;

    let (thumb_left, thumb_right, thumb_color) = if btn.is_active {
        // Khi BẬT: Nằm lệch hẳn sang PHẢI (track.right - 3 - 18), màu trắng sáng nổi bật
        let right = track_rect.right - 3;
        let left = right - 18;
        (left, right, rgb(255, 255, 255))
    } else {
        // Khi TẮT: Nằm lệch hẳn sang TRÁI (track.left + 3), màu xám trung tính #8B949E
        let left = track_rect.left + 3;
        let right = left + 18;
        let col = match btn.state {
            ButtonState::Hovered => rgb(170, 178, 188),
            _ => rgb(139, 148, 158),
        };
        (left, right, col)
    };

    let thumb_pen = CreatePen(PS_SOLID, 1, thumb_color);
    let thumb_brush = CreateSolidBrush(thumb_color);
    let old_pen = SelectObject(hdc, thumb_pen);
    let old_brush = SelectObject(hdc, thumb_brush);

    let _ = Ellipse(hdc, thumb_left, thumb_top, thumb_right, thumb_bottom);

    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(thumb_brush);
    let _ = DeleteObject(thumb_pen);

    // 4. Nhãn trạng thái bên trái: BẬT • 75°C (Cyan, Bold) / TẮT (Xám chìm, Normal)
    let label_rect = RECT {
        left: draw_rect.left + 2,
        top: draw_rect.top,
        right: track_rect.left - 8,
        bottom: draw_rect.bottom,
    };

    let (text_color, chosen_font) = if btn.is_active {
        (palette.cpu, font_bold)
    } else {
        (palette.text_sec, font_normal)
    };

    SetTextColor(hdc, text_color);
    SetBkMode(hdc, TRANSPARENT);
    let old_font = SelectObject(hdc, chosen_font);

    let mut mut_label_rect = label_rect;
    let mut wide_label: Vec<u16> = btn.label.encode_utf16().collect();
    let _ = DrawTextW(
        hdc,
        &mut wide_label,
        &mut mut_label_rect,
        DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    let _ = SelectObject(hdc, old_font);
}

/// Vẽ một nút bấm xúc giác với hiệu ứng nảy/lún và bo góc 10px
unsafe fn draw_tactile_button(
    hdc: HDC,
    btn: &TactileButton,
    palette: &ThemePalette,
    font_normal: HFONT,
    font_bold: HFONT,
) {
    if btn.id == ButtonId::ToggleEpp {
        draw_tactile_toggle_switch(hdc, btn, palette, font_normal, font_bold);
        return;
    }

    let mut draw_rect = btn.rect;

    // Hiệu ứng lún xúc giác (Tactile Sink): dịch chuyển phải +1px, xuống dưới +2px
    if btn.state == ButtonState::Pressed {
        draw_rect.left += 1;
        draw_rect.right += 1;
        draw_rect.top += 2;
        draw_rect.bottom += 2;
    }

    // Xác định màu sắc nền và viền theo trạng thái
    let (bg_color, border_color, border_width) = if btn.is_danger {
        match btn.state {
            ButtonState::Pressed => (palette.btn_pressed, palette.danger, 2),
            ButtonState::Hovered => (palette.btn_hover, palette.danger, 2),
            ButtonState::Normal => (palette.btn_norm, palette.border, 1),
        }
    } else if btn.is_active {
        match btn.state {
            ButtonState::Pressed => (palette.btn_pressed, palette.cpu, 2),
            ButtonState::Hovered => (palette.active_bg, palette.cpu, 2),
            ButtonState::Normal => (palette.active_bg, palette.cpu, 2),
        }
    } else {
        match btn.state {
            ButtonState::Pressed => (palette.btn_pressed, palette.border, 1),
            ButtonState::Hovered => (palette.btn_hover, palette.cpu, 1),
            ButtonState::Normal => (palette.btn_norm, palette.border, 1),
        }
    };

    let pen = CreatePen(PS_SOLID, border_width, border_color);
    let brush = CreateSolidBrush(bg_color);
    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, brush);

    // Bo góc 10px
    let _ = RoundRect(
        hdc,
        draw_rect.left,
        draw_rect.top,
        draw_rect.right,
        draw_rect.bottom,
        10,
        10,
    );

    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(brush);
    let _ = DeleteObject(pen);

    // Vẽ nhãn chữ giữa nút
    let text_color = if btn.is_danger && (btn.state == ButtonState::Hovered || btn.state == ButtonState::Pressed) {
        palette.danger
    } else if btn.is_active {
        palette.cpu
    } else {
        palette.text
    };

    SetTextColor(hdc, text_color);
    SetBkMode(hdc, TRANSPARENT);

    let chosen_font = if btn.is_active || btn.is_danger || btn.id == ButtonId::ToggleEpp {
        font_bold
    } else {
        font_normal
    };
    let old_font = SelectObject(hdc, chosen_font);

    let mut text_rect = draw_rect;
    let mut wide_label: Vec<u16> = btn.label.encode_utf16().collect();
    let _ = DrawTextW(
        hdc,
        &mut wide_label,
        &mut text_rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    let _ = SelectObject(hdc, old_font);
}

/// Vẽ toàn bộ nội dung Dashboard lên Memory DC (Double-Buffering thuần)
unsafe fn paint_dashboard(
    hwnd: HWND,
    hdc: HDC,
    width: i32,
    height: i32,
    state: &DashboardWindowState,
    layout: &DashboardLayout,
    snapshot: &DashboardSnapshot,
) {
    let palette = state.theme.palette();

    // 1. Tô nền toàn bộ cửa sổ
    let full_rect = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let bg_brush = CreateSolidBrush(palette.bg);
    FillRect(hdc, &full_rect, bg_brush);
    let _ = DeleteObject(bg_brush);

    SetBkMode(hdc, TRANSPARENT);

    // Khởi tạo các Font DPI-Aware sắc nét theo Tỷ lệ Vàng
    let font_title = create_dpi_font(hwnd, 15, FW_BOLD.0 as i32);
    let font_card_header = create_dpi_font_tenth(hwnd, 115, FW_BOLD.0 as i32); // 11.5pt Bold
    let font_temp = create_dpi_font(hwnd, 28, FW_BOLD.0 as i32); // Segoe UI 28pt Bold
    let font_sub_metric = create_dpi_font_tenth(hwnd, 105, FW_SEMIBOLD.0 as i32); // 10.5pt Semi-Bold
    let font_btn_normal = create_dpi_font_tenth(hwnd, 115, FW_SEMIBOLD.0 as i32); // 11.5pt Semi-Bold
    let font_btn_bold = create_dpi_font_tenth(hwnd, 115, FW_BOLD.0 as i32); // 11.5pt Bold

    // 2. Header Bar (Y: 10..40): Tiêu đề và thông báo hot-reload với clipping an toàn
    let mut title_right = width - 155;
    if snapshot.reload_flash_ticks > 0 && width >= 580 {
        let flash_rect = RECT {
            left: (width - 340).max(180),
            top: 10,
            right: width - 155,
            bottom: 40,
        };
        let _ = SelectObject(hdc, font_btn_bold);
        draw_text_clipped(
            hdc,
            flash_rect,
            "✓ Đã tải lại cấu hình!",
            rgb(76, 175, 80),
            DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        title_right = (flash_rect.left - 10).max(16);
    }
    let title_rect = RECT {
        left: 16,
        top: 10,
        right: title_right.max(16),
        bottom: 40,
    };
    let old_font = SelectObject(hdc, font_title);
    draw_text_clipped(
        hdc,
        title_rect,
        "PREDATOR FAN CONTROL // DASHBOARD",
        palette.text,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    // 3. Telemetry Cards (CPU & GPU) Multi-Line với Clipping Bounds an toàn
    draw_telemetry_card(
        hdc,
        layout.card_cpu_rect,
        "CPU TELEMETRY",
        snapshot.cpu_temp,
        snapshot.cpu_power_dw,
        snapshot.cpu_mhz,
        snapshot.cpu_speed,
        snapshot.cpu_rpm,
        palette.cpu,
        &palette,
        font_card_header,
        font_temp,
        font_sub_metric,
    );

    draw_telemetry_card(
        hdc,
        layout.card_gpu_rect,
        "GPU TELEMETRY",
        snapshot.gpu_temp,
        snapshot.gpu_power_dw,
        snapshot.gpu_mhz,
        snapshot.gpu_speed,
        snapshot.gpu_rpm,
        palette.gpu,
        &palette,
        font_card_header,
        font_temp,
        font_sub_metric,
    );

    // 3b. Khối hiển thị SYSTEM / MAINBOARD TELEMETRY STRIP (Lựa chọn B nâng cấp)
    draw_system_telemetry_strip(
        hdc,
        layout.strip_system_rect,
        snapshot.system_temp,
        &palette,
        font_card_header,
        font_btn_bold,
    );

    // 4. Panel Khóa nhiệt độ CPU (Smooth EPP) từ Layout động
    let epp_panel_rect = layout.epp_panel_rect;
    let epp_pen = CreatePen(PS_SOLID, 1, palette.border);
    let epp_brush = CreateSolidBrush(palette.card);
    let old_pen = SelectObject(hdc, epp_pen);
    let old_brush = SelectObject(hdc, epp_brush);

    let _ = RoundRect(
        hdc,
        epp_panel_rect.left,
        epp_panel_rect.top,
        epp_panel_rect.right,
        epp_panel_rect.bottom,
        10,
        10,
    );

    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(epp_brush);
    let _ = DeleteObject(epp_pen);

    // Tiêu đề EPP Governor với clipping không bao giờ đè nút Toggle:
    let epp_text_right = layout
        .buttons
        .iter()
        .find(|b| b.id == ButtonId::ToggleEpp)
        .map(|b| b.rect.left - 12)
        .unwrap_or(width - 170);

    let epp_text_rect = RECT {
        left: 28,
        top: epp_panel_rect.top,
        right: epp_text_right.max(28),
        bottom: epp_panel_rect.bottom,
    };
    let _ = SelectObject(hdc, font_btn_bold);
    draw_text_clipped(
        hdc,
        epp_text_rect,
        "Khóa nhiệt độ CPU (Smooth EPP Governor)",
        palette.text,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    // 5. Vẽ tất cả các nút bấm xúc giác và công tắc gạt
    for btn in &layout.buttons {
        if btn.id == ButtonId::ToggleEpp {
            draw_tactile_toggle_switch(hdc, btn, &palette, font_btn_normal, font_btn_bold);
        } else {
            draw_tactile_button(hdc, btn, &palette, font_btn_normal, font_btn_bold);
        }
    }

    // 6. Dọn dẹp tài nguyên GDI triệt để (Zero GDI Leak)
    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(font_title);
    let _ = DeleteObject(font_card_header);
    let _ = DeleteObject(font_temp);
    let _ = DeleteObject(font_sub_metric);
    let _ = DeleteObject(font_btn_normal);
    let _ = DeleteObject(font_btn_bold);
}

/// Window Procedure xử lý toàn bộ sự kiện của Dashboard Window
unsafe extern "system" fn dashboard_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            // Chống nhấp nháy 100%: bỏ qua lệnh vẽ nền mặc định của Win32
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

                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DashboardWindowState;
                if let Some(state) = ptr.as_ref() {
                    let snapshot = DashboardSnapshot::from_state(&state.state, state.reload_flash_ticks);
                    let layout = compute_dashboard_layout(
                        width,
                        height,
                        &snapshot,
                        state.theme,
                        state.hovered_btn,
                        state.pressed_btn,
                    );
                    paint_dashboard(hwnd, hdc_mem, width, height, state, &layout, &snapshot);
                }

                let _ = BitBlt(hdc_screen, 0, 0, width, height, hdc_mem, 0, 0, SRCCOPY);

                let _ = SelectObject(hdc_mem, old_bm);
                let _ = DeleteObject(hbm_mem);
                let _ = DeleteDC(hdc_mem);
            }

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == IDT_DASHBOARD_REFRESH {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DashboardWindowState;
                if let Some(state) = ptr.as_mut() {
                    if state.reload_flash_ticks > 0 {
                        state.reload_flash_ticks -= 1;
                    }
                }
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let pt = POINT { x, y };

            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DashboardWindowState;
            if let Some(state) = ptr.as_mut() {
                let mut client_rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut client_rect);
                let width = client_rect.right - client_rect.left;
                let height = client_rect.bottom - client_rect.top;

                let snapshot = DashboardSnapshot::from_state(&state.state, state.reload_flash_ticks);
                let buttons = compute_button_layout(
                    width,
                    height,
                    &snapshot,
                    state.theme,
                    state.hovered_btn,
                    state.pressed_btn,
                );

                let new_hovered = find_button_at(&buttons, pt);
                if new_hovered != state.hovered_btn {
                    state.hovered_btn = new_hovered;
                    let _ = InvalidateRect(hwnd, None, false);
                }

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
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let pt = POINT { x, y };

            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DashboardWindowState;
            if let Some(state) = ptr.as_mut() {
                let mut client_rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut client_rect);
                let width = client_rect.right - client_rect.left;
                let height = client_rect.bottom - client_rect.top;

                let snapshot = DashboardSnapshot::from_state(&state.state, state.reload_flash_ticks);
                let buttons = compute_button_layout(
                    width,
                    height,
                    &snapshot,
                    state.theme,
                    state.hovered_btn,
                    state.pressed_btn,
                );

                state.pressed_btn = find_button_at(&buttons, pt);
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let pt = POINT { x, y };

            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DashboardWindowState;
            if let Some(state) = ptr.as_mut() {
                let mut client_rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut client_rect);
                let width = client_rect.right - client_rect.left;
                let height = client_rect.bottom - client_rect.top;

                let snapshot = DashboardSnapshot::from_state(&state.state, state.reload_flash_ticks);
                let buttons = compute_button_layout(
                    width,
                    height,
                    &snapshot,
                    state.theme,
                    state.hovered_btn,
                    state.pressed_btn,
                );

                let released_btn = find_button_at(&buttons, pt);
                if let Some(pressed) = state.pressed_btn {
                    if released_btn == Some(pressed) {
                        handle_button_action(hwnd, pressed, state);
                    }
                }

                state.pressed_btn = None;
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DashboardWindowState;
            if let Some(state) = ptr.as_mut() {
                state.hovered_btn = None;
                state.pressed_btn = None;
                state.is_mouse_tracking = false;
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DashboardWindowState;
            if let Some(state) = ptr.as_ref() {
                if state.hovered_btn.is_some() {
                    let hand_cursor = LoadCursorW(None, IDC_HAND).unwrap_or_default();
                    if !hand_cursor.0.is_null() {
                        let _ = SetCursor(hand_cursor);
                        return LRESULT(1);
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_GETMINMAXINFO => {
            let p_mmi = lparam.0 as *mut MINMAXINFO;
            if let Some(mmi) = p_mmi.as_mut() {
                let win_dpi = GetDpiForWindow(hwnd);
                let dpi = if win_dpi == 0 {
                    let s = GetDpiForSystem();
                    if s == 0 { 96 } else { s as i32 }
                } else {
                    win_dpi as i32
                };
                mmi.ptMinTrackSize.x = MulDiv(560, dpi, 96);
                mmi.ptMinTrackSize.y = MulDiv(460, dpi, 96);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let suggested_rect = lparam.0 as *const RECT;
            if !suggested_rect.is_null() {
                let r = *suggested_rect;
                let _ = SetWindowPos(
                    hwnd,
                    HWND::default(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_SIZE => {
            if wparam.0 as u32 == SIZE_MINIMIZED {
                let _ = KillTimer(hwnd, IDT_DASHBOARD_REFRESH);
            } else {
                let _ = SetTimer(hwnd, IDT_DASHBOARD_REFRESH, 1000, None);
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // Đóng cửa sổ: Ẩn xuống khay hệ thống, ngắt Timer để CPU rơi về 0.00%
            let _ = KillTimer(hwnd, IDT_DASHBOARD_REFRESH);
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(hwnd, IDT_DASHBOARD_REFRESH);
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DashboardWindowState;
            if !ptr.is_null() {
                let _ = Box::from_raw(ptr);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            ACTIVE_DASHBOARD_HWND.store(0, Ordering::Relaxed);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Xử lý hành động khi người dùng nhấp nhả thành công một nút bấm
unsafe fn handle_button_action(hwnd: HWND, id: ButtonId, state: &mut DashboardWindowState) {
    match id {
        ButtonId::ProfileCurve => {
            state.state.set_profile(FanProfile::ActiveCurve);
            let _ = state.cmd_sender.send(TrayCommand::SetProfile(FanProfile::ActiveCurve));
        }
        ButtonId::ProfileTurbo => {
            state.state.set_profile(FanProfile::Turbo);
            let _ = state.cmd_sender.send(TrayCommand::SetProfile(FanProfile::Turbo));
        }
        ButtonId::ProfileSilent => {
            state.state.set_profile(FanProfile::Silent);
            let _ = state.cmd_sender.send(TrayCommand::SetProfile(FanProfile::Silent));
        }
        ButtonId::ToggleEpp => {
            let _ = state.state.toggle_governor();
            let _ = state.cmd_sender.send(TrayCommand::ToggleGovernor);
        }
        ButtonId::OpenCurveEditor => {
            crate::curve_dialog::open_curve_dialog(state.cmd_sender.clone(), Some(state.state.clone()));
        }
        ButtonId::OpenTelemetryChart => {
            crate::chart_window::show_or_focus_chart_window(state.telemetry.clone());
        }
        ButtonId::ReloadConfig => {
            let _ = state.cmd_sender.send(TrayCommand::ReloadConfig);
            state.reload_flash_ticks = 3;
        }
        ButtonId::ExitApp => {
            let _ = state.cmd_sender.send(TrayCommand::Exit);
            let _ = ShowWindow(hwnd, SW_HIDE);
            PostQuitMessage(0);
        }
        ButtonId::ToggleTheme => {
            state.theme = match state.theme {
                AppTheme::Dark => AppTheme::Light,
                AppTheme::Light => AppTheme::Dark,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_macro() {
        assert_eq!(rgb(18, 20, 23), COLORREF(0x00171412));
        assert_eq!(rgb(0, 240, 255), COLORREF(0x00FFF000));
        assert_eq!(rgb(255, 167, 38), COLORREF(0x0026A7FF));
        assert_eq!(rgb(255, 59, 48), COLORREF(0x00303BFF));
    }

    #[test]
    fn test_theme_palettes() {
        let dark = AppTheme::Dark.palette();
        assert_eq!(dark.bg, rgb(18, 20, 23));
        assert_eq!(dark.cpu, rgb(0, 240, 255));
        assert_eq!(dark.gpu, rgb(255, 167, 38));
        assert_eq!(dark.text, rgb(248, 250, 252));
        assert_eq!(dark.text_sec, rgb(208, 217, 230));

        let light = AppTheme::Light.palette();
        assert_eq!(light.bg, rgb(244, 246, 249));
        assert_eq!(light.cpu, rgb(0, 151, 167));
        assert_eq!(light.gpu, rgb(230, 81, 0));
    }

    #[test]
    fn test_rect_contains_and_hit_testing() {
        let rect = RECT {
            left: 10,
            top: 20,
            right: 100,
            bottom: 60,
        };

        assert!(rect_contains(&rect, POINT { x: 10, y: 20 }));
        assert!(rect_contains(&rect, POINT { x: 50, y: 40 }));
        assert!(rect_contains(&rect, POINT { x: 99, y: 59 }));
        assert!(!rect_contains(&rect, POINT { x: 100, y: 60 }));
        assert!(!rect_contains(&rect, POINT { x: 9, y: 20 }));
        assert!(!rect_contains(&rect, POINT { x: 50, y: 61 }));
    }

    #[test]
    fn test_button_layout_generation() {
        let snapshot = DashboardSnapshot {
            cpu_temp: 65,
            cpu_speed: 45,
            gpu_temp: 58,
            gpu_speed: 40,
            cpu_rpm: 3200,
            gpu_rpm: 3100,
            cpu_power_dw: 420,
            gpu_power_dw: 185,
            cpu_mhz: 3800,
            gpu_mhz: 2200,
            system_temp: 54,
            active_profile: FanProfile::Turbo,
            governor_enabled: true,
            governor_target_temp: 80,
            governor_epp: 96,
            reload_flash_ticks: 0,
        };

        let buttons = compute_button_layout(
            524,
            411,
            &snapshot,
            AppTheme::Dark,
            Some(ButtonId::ProfileTurbo),
            Some(ButtonId::ProfileTurbo),
        );

        assert_eq!(buttons.len(), 9);

        // Nút Giao diện phải có chiều rộng 135px, neo ở w - 145, top: 10, bottom: 40
        let theme_btn = buttons.iter().find(|b| b.id == ButtonId::ToggleTheme).unwrap();
        assert_eq!(theme_btn.rect.left, 524 - 145);
        assert_eq!(theme_btn.rect.right, 524 - 10);
        assert_eq!(theme_btn.rect.right - theme_btn.rect.left, 135);
        assert_eq!(theme_btn.rect.top, 10);
        assert_eq!(theme_btn.rect.bottom, 40);

        // Nút Turbo phải có state Pressed và is_active = true, chiều cao từ 38 đến 44 px
        let turbo_btn = buttons.iter().find(|b| b.id == ButtonId::ProfileTurbo).unwrap();
        assert_eq!(turbo_btn.state, ButtonState::Pressed);
        assert!(turbo_btn.is_active);
        let turbo_h = turbo_btn.rect.bottom - turbo_btn.rect.top;
        assert!((38..=44).contains(&turbo_h), "Turbo button height {} not in 38..44", turbo_h);

        // Nút Curve phải có state Normal và is_active = false
        let curve_btn = buttons.iter().find(|b| b.id == ButtonId::ProfileCurve).unwrap();
        assert_eq!(curve_btn.state, ButtonState::Normal);
        assert!(!curve_btn.is_active);

        // Nút Toggle EPP (Pill Toggle Switch) phải có is_active = true, label chứa 80°C, width 148px, height 32px
        let epp_btn = buttons.iter().find(|b| b.id == ButtonId::ToggleEpp).unwrap();
        assert!(epp_btn.is_active);
        assert!(epp_btn.label.contains("80°C"));
        assert_eq!(epp_btn.rect.right - epp_btn.rect.left, 148);
        assert_eq!(epp_btn.rect.bottom - epp_btn.rect.top, 32);

        // Nút ExitApp phải có is_danger = true, chiều cao từ 38 đến 44 px
        let exit_btn = buttons.iter().find(|b| b.id == ButtonId::ExitApp).unwrap();
        assert!(exit_btn.is_danger);
        let exit_h = exit_btn.rect.bottom - exit_btn.rect.top;
        assert!((38..=44).contains(&exit_h), "Exit button height {} not in 38..44", exit_h);
    }

    #[test]
    fn test_tactile_toggle_switch_layout_and_states() {
        let mut snap_on = DashboardSnapshot::default();
        snap_on.governor_enabled = true;
        snap_on.governor_target_temp = 75;

        let mut snap_off = DashboardSnapshot::default();
        snap_off.governor_enabled = false;
        snap_off.governor_target_temp = 75;

        // 1. Kiểm tra trạng thái BẬT (ON)
        let btns_on = compute_button_layout(600, 500, &snap_on, AppTheme::Dark, None, None);
        let toggle_on = btns_on.iter().find(|b| b.id == ButtonId::ToggleEpp).unwrap();
        assert!(toggle_on.is_active);
        assert_eq!(toggle_on.label, "BẬT • 75°C");
        assert_eq!(toggle_on.rect.right - toggle_on.rect.left, 148);
        assert_eq!(toggle_on.rect.bottom - toggle_on.rect.top, 32);

        // 2. Kiểm tra trạng thái TẮT (OFF)
        let btns_off = compute_button_layout(600, 500, &snap_off, AppTheme::Dark, None, None);
        let toggle_off = btns_off.iter().find(|b| b.id == ButtonId::ToggleEpp).unwrap();
        assert!(!toggle_off.is_active);
        assert_eq!(toggle_off.label, "TẮT");
        assert_eq!(toggle_off.rect.right - toggle_off.rect.left, 148);
        assert_eq!(toggle_off.rect.bottom - toggle_off.rect.top, 32);

        // 3. Kiểm tra Hit-testing bao phủ toàn bộ cụm: nhãn chữ bên trái và thanh gạt bên phải
        // Điểm ở góc trái (vùng nhãn chữ)
        let pt_label = POINT { x: toggle_on.rect.left + 15, y: toggle_on.rect.top + 16 };
        assert_eq!(find_button_at(&btns_on, pt_label), Some(ButtonId::ToggleEpp));

        // Điểm ở góc phải (vùng thanh gạt / track)
        let pt_track = POINT { x: toggle_on.rect.right - 15, y: toggle_on.rect.top + 16 };
        assert_eq!(find_button_at(&btns_on, pt_track), Some(ButtonId::ToggleEpp));

        // Điểm ở vị trí chính giữa
        let pt_center = POINT {
            x: (toggle_on.rect.left + toggle_on.rect.right) / 2,
            y: toggle_on.rect.top + 16,
        };
        assert_eq!(find_button_at(&btns_on, pt_center), Some(ButtonId::ToggleEpp));

        // Điểm ngay sát ngoài biên phải (không trúng)
        let pt_outside = POINT { x: toggle_on.rect.right + 2, y: toggle_on.rect.top + 16 };
        assert_ne!(find_button_at(&btns_on, pt_outside), Some(ButtonId::ToggleEpp));
    }

    #[test]
    fn test_dashboard_snapshot_from_state() {
        let state = Arc::new(TrayState::new());
        state.update_status(72, 60, 68, 55, 3600, 3400);
        state.update_metrics(428, 195, 3820, 2250);
        state.update_governor_status(true, 85, 128);
        state.set_profile(FanProfile::Silent);

        let snap = DashboardSnapshot::from_state(&state, 2);
        assert_eq!(snap.cpu_temp, 72);
        assert_eq!(snap.cpu_speed, 60);
        assert_eq!(snap.gpu_temp, 68);
        assert_eq!(snap.gpu_speed, 55);
        assert_eq!(snap.cpu_rpm, 3600);
        assert_eq!(snap.gpu_rpm, 3400);
        assert_eq!(snap.cpu_power_dw, 428);
        assert_eq!(snap.gpu_power_dw, 195);
        assert_eq!(snap.cpu_mhz, 3820);
        assert_eq!(snap.gpu_mhz, 2250);
        assert_eq!(snap.active_profile, FanProfile::Silent);
        assert!(snap.governor_enabled);
        assert_eq!(snap.governor_target_temp, 85);
        assert_eq!(snap.governor_epp, 128);
        assert_eq!(snap.reload_flash_ticks, 2);
    }

    #[test]
    fn test_dashboard_snapshot_metrics_formatting() {
        let snap = DashboardSnapshot {
            cpu_power_dw: 428,
            gpu_power_dw: 192,
            cpu_mhz: 3820,
            gpu_mhz: 2250,
            ..DashboardSnapshot::default()
        };

        // Format cũ
        let cpu_pwr = snap.cpu_power_dw as f32 / 10.0;
        let cpu_ghz = snap.cpu_mhz as f32 / 1000.0;
        let str_full = format!("Công suất: {:.1} W | Xung nhịp: {:.2} GHz", cpu_pwr, cpu_ghz);
        assert_eq!(str_full, "Công suất: 42.8 W | Xung nhịp: 3.82 GHz");

        // Format Multi-Line mới theo Kiến trúc Sư trưởng (Phase 1)
        let pwr_str = format!("⚡ Công suất: {:.1} W", cpu_pwr);
        assert_eq!(pwr_str, "⚡ Công suất: 42.8 W");
        let clk_str = format!("⏱ Xung nhịp: {:.2} GHz", cpu_ghz);
        assert_eq!(clk_str, "⏱ Xung nhịp: 3.82 GHz");

        let gpu_pwr = snap.gpu_power_dw as f32 / 10.0;
        let gpu_ghz = snap.gpu_mhz as f32 / 1000.0;
        let gpu_str_full = format!("Công suất: {:.1} W | Xung nhịp: {:.2} GHz", gpu_pwr, gpu_ghz);
        assert_eq!(gpu_str_full, "Công suất: 19.2 W | Xung nhịp: 2.25 GHz");

        let gpu_pwr_str = format!("⚡ Công suất: {:.1} W", gpu_pwr);
        assert_eq!(gpu_pwr_str, "⚡ Công suất: 19.2 W");
        let gpu_clk_str = format!("⏱ Xung nhịp: {:.2} GHz", gpu_ghz);
        assert_eq!(gpu_clk_str, "⏱ Xung nhịp: 2.25 GHz");
    }

    #[test]
    fn test_responsive_resizing_layout() {
        let snapshot = DashboardSnapshot {
            cpu_temp: 70,
            cpu_speed: 50,
            gpu_temp: 65,
            gpu_speed: 45,
            cpu_rpm: 3400,
            gpu_rpm: 3200,
            cpu_power_dw: 450,
            gpu_power_dw: 200,
            cpu_mhz: 4000,
            gpu_mhz: 2300,
            system_temp: 54,
            active_profile: FanProfile::ActiveCurve,
            governor_enabled: true,
            governor_target_temp: 85,
            governor_epp: 96,
            reload_flash_ticks: 0,
        };

        // Kiểm tra 4 kích thước: nhỏ nhất (480x440), cũ (540x480), chuẩn mới (600x548), và mở rộng lớn (1000x800)
        let sizes = [(480, 440), (540, 480), (600, 548), (1000, 800)];

        for &(w, h) in &sizes {
            let layout = compute_dashboard_layout(
                w,
                h,
                &snapshot,
                AppTheme::Dark,
                None,
                None,
            );

            // 1. Kiểm tra 2 card CPU và GPU
            assert!(layout.card_cpu_rect.left < layout.card_cpu_rect.right);
            assert!(layout.card_cpu_rect.top < layout.card_cpu_rect.bottom);
            assert!(layout.card_gpu_rect.left < layout.card_gpu_rect.right);
            assert!(layout.card_gpu_rect.top < layout.card_gpu_rect.bottom);

            // Card CPU nằm bên trái Card GPU và không đè lên nhau
            assert!(layout.card_cpu_rect.right < layout.card_gpu_rect.left);

            // Cả 2 card nằm trong giới hạn màn hình
            assert!(layout.card_cpu_rect.left >= 16);
            assert!(layout.card_gpu_rect.right <= w - 16);

            // 2. Kiểm tra System Telemetry Strip nằm giữa 2 card và Profiles Buttons
            assert!(layout.strip_system_rect.left >= 16);
            assert!(layout.strip_system_rect.right <= w - 16);
            assert!(layout.strip_system_rect.top >= layout.card_cpu_rect.bottom);
            let prof_btn = layout.buttons.iter().find(|b| b.id == ButtonId::ProfileCurve).unwrap();
            assert!(layout.strip_system_rect.bottom <= prof_btn.rect.top);

            // 3. Kiểm tra EPP Panel
            assert!(layout.epp_panel_rect.left >= 16);
            assert!(layout.epp_panel_rect.right <= w - 16);
            assert!(layout.epp_panel_rect.top >= prof_btn.rect.bottom);
            assert!(layout.epp_panel_rect.bottom <= h);

            // 4. Kiểm tra số lượng và tính hợp lệ của tất cả các nút
            assert_eq!(layout.buttons.len(), 9);
            for btn in &layout.buttons {
                assert!(btn.rect.left < btn.rect.right, "Nút {:?} có width <= 0", btn.id);
                assert!(btn.rect.top < btn.rect.bottom, "Nút {:?} có height <= 0", btn.id);
                assert!(btn.rect.left >= 0 && btn.rect.right <= w, "Nút {:?} vượt biên ngang", btn.id);
                assert!(btn.rect.top >= 0 && btn.rect.bottom <= h, "Nút {:?} vượt biên dọc", btn.id);
            }

            // 5. Kiểm tra Flex Layout: hàng nút cuối (ExitApp) chạm đúng đáy h - 16, không có khoảng trống đen hổng
            let exit_btn = layout.buttons.iter().find(|b| b.id == ButtonId::ExitApp).unwrap();
            assert_eq!(exit_btn.rect.bottom, h - 16, "Hàng nút đáy phải chạm đúng h - 16");
        }
    }

    #[test]
    fn test_dashboard_window_creation() {
        let (_sender, _receiver) = std::sync::mpsc::channel::<crate::tray::TrayCommand>();
        let _state = Arc::new(crate::tray::TrayState::new());
        let _telemetry = Arc::new(Mutex::new(TelemetryHistory::new()));
        
        unsafe {
            let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap();
            let class_name = w!("PredatorDashboardWndClassTest");
            let wnd_class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(dashboard_wndproc),
                hInstance: hinstance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
                lpszClassName: class_name,
                ..Default::default()
            };
            let _ = RegisterClassW(&wnd_class);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                w!("Test Dashboard"),
                WS_OVERLAPPEDWINDOW,
                100, 100, 600, 548,
                HWND::default(),
                HMENU::default(),
                hinstance,
                None,
            );
            assert!(hwnd.is_ok(), "CreateWindowExW must succeed: {:?}", hwnd.err());
            let h = hwnd.unwrap();
            assert!(!h.0.is_null());
            let _ = DestroyWindow(h);
        }
    }
}
