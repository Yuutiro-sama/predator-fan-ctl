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
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE};
use windows::Win32::UI::WindowsAndMessaging::*;

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
                cpu: rgb(0, 229, 255),        // #00E5FF
                gpu: rgb(255, 145, 0),        // #FF9100
                text: rgb(236, 239, 244),     // #ECEFF4
                text_sec: rgb(139, 148, 158), // #8B949E
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
    let h = height.max(400);

    let get_state = |id: ButtonId| -> ButtonState {
        if pressed == Some(id) {
            ButtonState::Pressed
        } else if hovered == Some(id) {
            ButtonState::Hovered
        } else {
            ButtonState::Normal
        }
    };

    // 1. Nút đổi Giao diện (Header Theme Toggle)
    let theme_btn_rect = RECT {
        left: w - 110,
        top: 10,
        right: w - 16,
        bottom: 38,
    };

    // Phân bổ chiều dọc linh hoạt (Responsive Vertical Distribution)
    let available_content = (h - 48 - 16).max(340);
    let card_h = (available_content * 35 / 100).clamp(138, 240);
    let card_bottom = 48 + card_h;

    let base_gap = ((available_content - (card_h + 38 + 38 + 78)) / 4).clamp(6, 20);

    let prof_y_top = card_bottom + base_gap;
    let prof_h = (available_content * 9 / 100).clamp(36, 48);
    let prof_y_bottom = prof_y_top + prof_h;

    let epp_y_top = prof_y_bottom + base_gap;
    let epp_h = prof_h;
    let epp_y_bottom = epp_y_top + epp_h;

    let tools_y_top = epp_y_bottom + base_gap;
    let tool_row_h = prof_h;
    let tool_gap = (base_gap / 2).max(4);
    let tool_r1_top = tools_y_top;
    let tool_r1_bottom = tool_r1_top + tool_row_h;
    let tool_r2_top = tool_r1_bottom + tool_gap;
    let tool_r2_bottom = tool_r2_top + tool_row_h;

    // --- Cards Rect ---
    let card_gap = 12;
    let card_w = (w - 32 - card_gap) / 2;
    let card_cpu_rect = RECT {
        left: 16,
        top: 48,
        right: 16 + card_w,
        bottom: card_bottom,
    };
    let card_gpu_rect = RECT {
        left: 16 + card_w + card_gap,
        top: 48,
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
    let epp_btn_h = 28.min(epp_h - 8);
    let epp_btn_top = epp_y_top + (epp_h - epp_btn_h) / 2;
    let epp_btn_rect = RECT {
        left: w - 16 - 130,
        top: epp_btn_top,
        right: w - 24,
        bottom: epp_btn_top + epp_btn_h,
    };
    let epp_label = if snapshot.governor_enabled {
        format!("BẬT ({}°C)", snapshot.governor_target_temp)
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

        // Kích thước chuẩn 540 x 450 px, căn giữa màn hình
        let width = 540;
        let height = 450;
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let pos_x = (screen_w - width) / 2;
        let pos_y = (screen_h - height) / 2;

        let window_style = WS_OVERLAPPEDWINDOW;

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
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

/// Helper tạo HFONT Segoe UI
unsafe fn create_font(height: i32, weight: i32) -> HFONT {
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

/// Vẽ một nút bấm xúc giác với hiệu ứng nảy/lún và bo góc 10px
unsafe fn draw_tactile_button(
    hdc: HDC,
    btn: &TactileButton,
    palette: &ThemePalette,
    font_normal: HFONT,
    font_bold: HFONT,
) {
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

    let chosen_font = if btn.is_active || btn.is_danger {
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
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    let _ = SelectObject(hdc, old_font);
}

/// Vẽ toàn bộ nội dung Dashboard lên Memory DC (Double-Buffering thuần)
unsafe fn paint_dashboard(
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

    // Khởi tạo các Font cần thiết
    let font_title = create_font(17, FW_BOLD.0 as i32);
    let font_card_header = create_font(14, FW_BOLD.0 as i32);
    let font_temp = create_font(36, FW_BOLD.0 as i32); // 27pt Segoe UI Bold
    let font_small = create_font(13, FW_NORMAL.0 as i32);
    let font_body = create_font(14, FW_NORMAL.0 as i32);
    let font_body_bold = create_font(14, FW_SEMIBOLD.0 as i32);

    // 2. Header Bar (Y: 10..42)
    let old_font = SelectObject(hdc, font_title);
    draw_text_line(hdc, 16, 14, "PREDATOR FAN CONTROL // DASHBOARD", palette.text);

    // Thông báo trạng thái hot-reload config (nếu có nháy flash)
    if snapshot.reload_flash_ticks > 0 {
        let _ = SelectObject(hdc, font_body_bold);
        draw_text_line(hdc, 270, 16, "✓ Đã tải lại cấu hình!", rgb(76, 175, 80));
    }

    // 3. Telemetry Cards (CPU & GPU) từ Layout động
    let cpu_card_rect = layout.card_cpu_rect;
    let card_pen = CreatePen(PS_SOLID, 1, palette.border);
    let card_brush = CreateSolidBrush(palette.card);
    let old_pen = SelectObject(hdc, card_pen);
    let old_brush = SelectObject(hdc, card_brush);

    let _ = RoundRect(
        hdc,
        cpu_card_rect.left,
        cpu_card_rect.top,
        cpu_card_rect.right,
        cpu_card_rect.bottom,
        10,
        10,
    );

    // --- GPU Card ---
    let gpu_card_rect = layout.card_gpu_rect;
    let _ = RoundRect(
        hdc,
        gpu_card_rect.left,
        gpu_card_rect.top,
        gpu_card_rect.right,
        gpu_card_rect.bottom,
        10,
        10,
    );

    // Khôi phục pen & brush của card
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(card_brush);
    let _ = DeleteObject(card_pen);

    // Nội dung CPU Card
    let _ = SelectObject(hdc, font_card_header);
    draw_text_line(hdc, cpu_card_rect.left + 14, cpu_card_rect.top + 12, "CPU TELEMETRY", palette.cpu);

    let _ = SelectObject(hdc, font_temp);
    let cpu_temp_str = format!("{}°C", snapshot.cpu_temp);
    draw_text_line(hdc, cpu_card_rect.left + 14, cpu_card_rect.top + 32, &cpu_temp_str, palette.cpu);

    // Dòng phụ: "Công suất: XX.X W | Xung nhịp: X.XX GHz"
    let _ = SelectObject(hdc, font_small);
    let cpu_pwr = snapshot.cpu_power_dw as f32 / 10.0;
    let cpu_ghz = snapshot.cpu_mhz as f32 / 1000.0;
    let cpu_card_w = cpu_card_rect.right - cpu_card_rect.left;
    let cpu_metric_str = if cpu_card_w >= 235 {
        format!("Công suất: {:.1} W | Xung nhịp: {:.2} GHz", cpu_pwr, cpu_ghz)
    } else {
        format!("Công suất: {:.1}W | {:.2}GHz", cpu_pwr, cpu_ghz)
    };
    let cpu_card_h = cpu_card_rect.bottom - cpu_card_rect.top;
    let metric_rel_y = (cpu_card_h * 50 / 100).clamp(68, 100);
    draw_text_line(hdc, cpu_card_rect.left + 14, cpu_card_rect.top + metric_rel_y, &cpu_metric_str, palette.text_sec);

    // Thanh Bar quạt CPU (Tính toán vị trí tương đối theo chiều cao card)
    let track_rel_y = (cpu_card_h * 64 / 100).max(88);
    let cpu_track_rect = RECT {
        left: cpu_card_rect.left + 14,
        top: cpu_card_rect.top + track_rel_y,
        right: cpu_card_rect.right - 14,
        bottom: cpu_card_rect.top + track_rel_y + 10,
    };
    let track_brush = CreateSolidBrush(palette.bar_bg);
    FillRect(hdc, &cpu_track_rect, track_brush);

    let cpu_track_w = cpu_track_rect.right - cpu_track_rect.left;
    let cpu_fill_w = (cpu_track_w * (snapshot.cpu_speed.min(100) as i32)) / 100;
    if cpu_fill_w > 0 {
        let cpu_fill_rect = RECT {
            left: cpu_track_rect.left,
            top: cpu_track_rect.top,
            right: cpu_track_rect.left + cpu_fill_w,
            bottom: cpu_track_rect.bottom,
        };
        let cpu_fill_brush = CreateSolidBrush(palette.cpu);
        FillRect(hdc, &cpu_fill_rect, cpu_fill_brush);
        let _ = DeleteObject(cpu_fill_brush);
    }

    let _ = SelectObject(hdc, font_small);
    let cpu_info = if snapshot.cpu_rpm > 0 {
        format!("Quạt: {}% | {} RPM", snapshot.cpu_speed, snapshot.cpu_rpm)
    } else {
        format!("Quạt: {}%", snapshot.cpu_speed)
    };
    draw_text_line(hdc, cpu_card_rect.left + 14, cpu_card_rect.top + track_rel_y + 16, &cpu_info, palette.text_sec);

    // Nội dung GPU Card
    let _ = SelectObject(hdc, font_card_header);
    draw_text_line(hdc, gpu_card_rect.left + 14, gpu_card_rect.top + 12, "GPU TELEMETRY", palette.gpu);

    let _ = SelectObject(hdc, font_temp);
    let gpu_temp_str = format!("{}°C", snapshot.gpu_temp);
    draw_text_line(hdc, gpu_card_rect.left + 14, gpu_card_rect.top + 32, &gpu_temp_str, palette.gpu);

    // Dòng phụ: "Công suất: XX.X W | Xung nhịp: X.XX GHz"
    let _ = SelectObject(hdc, font_small);
    let gpu_pwr = snapshot.gpu_power_dw as f32 / 10.0;
    let gpu_ghz = snapshot.gpu_mhz as f32 / 1000.0;
    let gpu_card_w = gpu_card_rect.right - gpu_card_rect.left;
    let gpu_metric_str = if gpu_card_w >= 235 {
        format!("Công suất: {:.1} W | Xung nhịp: {:.2} GHz", gpu_pwr, gpu_ghz)
    } else {
        format!("Công suất: {:.1}W | {:.2}GHz", gpu_pwr, gpu_ghz)
    };
    let gpu_card_h = gpu_card_rect.bottom - gpu_card_rect.top;
    let gpu_metric_rel_y = (gpu_card_h * 50 / 100).clamp(68, 100);
    draw_text_line(hdc, gpu_card_rect.left + 14, gpu_card_rect.top + gpu_metric_rel_y, &gpu_metric_str, palette.text_sec);

    // Thanh Bar quạt GPU
    let gpu_track_rel_y = (gpu_card_h * 64 / 100).max(88);
    let gpu_track_rect = RECT {
        left: gpu_card_rect.left + 14,
        top: gpu_card_rect.top + gpu_track_rel_y,
        right: gpu_card_rect.right - 14,
        bottom: gpu_card_rect.top + gpu_track_rel_y + 10,
    };
    FillRect(hdc, &gpu_track_rect, track_brush);
    let _ = DeleteObject(track_brush);

    let gpu_track_w = gpu_track_rect.right - gpu_track_rect.left;
    let gpu_fill_w = (gpu_track_w * (snapshot.gpu_speed.min(100) as i32)) / 100;
    if gpu_fill_w > 0 {
        let gpu_fill_rect = RECT {
            left: gpu_track_rect.left,
            top: gpu_track_rect.top,
            right: gpu_track_rect.left + gpu_fill_w,
            bottom: gpu_track_rect.bottom,
        };
        let gpu_fill_brush = CreateSolidBrush(palette.gpu);
        FillRect(hdc, &gpu_fill_rect, gpu_fill_brush);
        let _ = DeleteObject(gpu_fill_brush);
    }

    let _ = SelectObject(hdc, font_small);
    let gpu_info = if snapshot.gpu_rpm > 0 {
        format!("Quạt: {}% | {} RPM", snapshot.gpu_speed, snapshot.gpu_rpm)
    } else {
        format!("Quạt: {}%", snapshot.gpu_speed)
    };
    draw_text_line(hdc, gpu_card_rect.left + 14, gpu_card_rect.top + gpu_track_rel_y + 16, &gpu_info, palette.text_sec);

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

    let epp_h = epp_panel_rect.bottom - epp_panel_rect.top;
    let epp_text_y = epp_panel_rect.top + (epp_h - 16) / 2;
    let _ = SelectObject(hdc, font_body_bold);
    draw_text_line(
        hdc,
        epp_panel_rect.left + 14,
        epp_text_y,
        "Khóa nhiệt độ CPU (Smooth EPP Governor)",
        palette.text,
    );

    // 5. Vẽ tất cả các nút bấm xúc giác
    for btn in &layout.buttons {
        draw_tactile_button(hdc, btn, &palette, font_body, font_body_bold);
    }

    // 6. Dọn dẹp tài nguyên GDI chặt chẽ (Zero-Leak)
    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(font_title);
    let _ = DeleteObject(font_card_header);
    let _ = DeleteObject(font_temp);
    let _ = DeleteObject(font_small);
    let _ = DeleteObject(font_body);
    let _ = DeleteObject(font_body_bold);
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
                    paint_dashboard(hdc_mem, width, height, state, &layout, &snapshot);
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
        WM_GETMINMAXINFO => {
            let p_mmi = lparam.0 as *mut MINMAXINFO;
            if let Some(mmi) = p_mmi.as_mut() {
                mmi.ptMinTrackSize.x = 480;
                mmi.ptMinTrackSize.y = 420;
            }
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
        assert_eq!(rgb(0, 229, 255), COLORREF(0x00FFE500));
        assert_eq!(rgb(255, 145, 0), COLORREF(0x000091FF));
        assert_eq!(rgb(255, 59, 48), COLORREF(0x00303BFF));
    }

    #[test]
    fn test_theme_palettes() {
        let dark = AppTheme::Dark.palette();
        assert_eq!(dark.bg, rgb(18, 20, 23));
        assert_eq!(dark.cpu, rgb(0, 229, 255));
        assert_eq!(dark.gpu, rgb(255, 145, 0));

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

        // Nút Turbo phải có state Pressed và is_active = true
        let turbo_btn = buttons.iter().find(|b| b.id == ButtonId::ProfileTurbo).unwrap();
        assert_eq!(turbo_btn.state, ButtonState::Pressed);
        assert!(turbo_btn.is_active);

        // Nút Curve phải có state Normal và is_active = false
        let curve_btn = buttons.iter().find(|b| b.id == ButtonId::ProfileCurve).unwrap();
        assert_eq!(curve_btn.state, ButtonState::Normal);
        assert!(!curve_btn.is_active);

        // Nút Toggle EPP phải có is_active = true và label chứa 80°C
        let epp_btn = buttons.iter().find(|b| b.id == ButtonId::ToggleEpp).unwrap();
        assert!(epp_btn.is_active);
        assert!(epp_btn.label.contains("80°C"));

        // Nút ExitApp phải có is_danger = true
        let exit_btn = buttons.iter().find(|b| b.id == ButtonId::ExitApp).unwrap();
        assert!(exit_btn.is_danger);
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

        let cpu_pwr = snap.cpu_power_dw as f32 / 10.0;
        let cpu_ghz = snap.cpu_mhz as f32 / 1000.0;
        let str_full = format!("Công suất: {:.1} W | Xung nhịp: {:.2} GHz", cpu_pwr, cpu_ghz);
        assert_eq!(str_full, "Công suất: 42.8 W | Xung nhịp: 3.82 GHz");

        let gpu_pwr = snap.gpu_power_dw as f32 / 10.0;
        let gpu_ghz = snap.gpu_mhz as f32 / 1000.0;
        let gpu_str_full = format!("Công suất: {:.1} W | Xung nhịp: {:.2} GHz", gpu_pwr, gpu_ghz);
        assert_eq!(gpu_str_full, "Công suất: 19.2 W | Xung nhịp: 2.25 GHz");
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
            active_profile: FanProfile::ActiveCurve,
            governor_enabled: true,
            governor_target_temp: 85,
            governor_epp: 96,
            reload_flash_ticks: 0,
        };

        // Kiểm tra 3 kích thước: nhỏ nhất (480x420), chuẩn (540x450), và mở rộng lớn (1000x800)
        let sizes = [(480, 420), (540, 450), (1000, 800)];

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

            // 2. Kiểm tra EPP Panel
            assert!(layout.epp_panel_rect.left >= 16);
            assert!(layout.epp_panel_rect.right <= w - 16);
            assert!(layout.epp_panel_rect.top >= layout.card_cpu_rect.bottom);
            assert!(layout.epp_panel_rect.bottom <= h);

            // 3. Kiểm tra số lượng và tính hợp lệ của tất cả các nút
            assert_eq!(layout.buttons.len(), 9);
            for btn in &layout.buttons {
                assert!(btn.rect.left < btn.rect.right, "Nút {:?} có width <= 0", btn.id);
                assert!(btn.rect.top < btn.rect.bottom, "Nút {:?} có height <= 0", btn.id);
                assert!(btn.rect.left >= 0 && btn.rect.right <= w, "Nút {:?} vượt biên ngang", btn.id);
                assert!(btn.rect.top >= 0 && btn.rect.bottom <= h, "Nút {:?} vượt biên dọc", btn.id);
            }
        }
    }
}
