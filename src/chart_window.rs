//! Acer Predator Temperature History Chart Window (Pure Win32 GDI Dark Mode)
//!
//! Cửa sổ hiển thị biểu đồ nhiệt độ CPU và GPU trong 1 giờ gần nhất:
//! - Giao diện Dark Mode chuẩn Predator Theme (#121417).
//! - Hoàn toàn bằng Win32 GDI thuần, Double-Buffering chống nhấp nháy (Flicker-Free).
//! - Trục Y: Cố định 30°C - 100°C với lưới chia 10°C và đường cảnh báo 85°C.
//! - Trục X: 1 giờ (3600 giây) chia mốc mỗi 10 phút (-60m .. Now).
//! - Đường biểu diễn CPU (Cyan #00E5FF) và GPU (Orange #FF9100) độ dày 2px.
//! - Tương tác chuột: Crosshair dọc và Tooltip hiển thị thời gian và nhiệt độ chi tiết.
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

/// Màu sắc theo bảng màu Predator Dark Theme
#[inline]
const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

const COLOR_BG: COLORREF = rgb(18, 20, 23);         // #121417
const COLOR_PLOT_BG: COLORREF = rgb(22, 25, 30);    // #16191E
const COLOR_BORDER: COLORREF = rgb(52, 59, 72);     // #343B48
const COLOR_GRID: COLORREF = rgb(40, 44, 52);       // #282C34
const COLOR_WARN_85: COLORREF = rgb(229, 57, 53);   // #E53935
const COLOR_CPU: COLORREF = rgb(0, 229, 255);       // #00E5FF (Cyan)
const COLOR_GPU: COLORREF = rgb(255, 145, 0);       // #FF9100 (Orange)
const COLOR_TEXT_MUTED: COLORREF = rgb(136, 146, 176); // #8892B0
const COLOR_TEXT_LIGHT: COLORREF = rgb(230, 237, 243); // #E6EDF3
const COLOR_CROSSHAIR: COLORREF = rgb(92, 99, 112); // #5C6370
const COLOR_TOOLTIP_BG: COLORREF = rgb(26, 29, 36); // #1A1D24
const COLOR_TOOLTIP_BORDER: COLORREF = rgb(70, 78, 92); // #464E5C

/// Trạng thái dữ liệu nội bộ gắn với HWND
struct ChartWindowState {
    telemetry: Arc<Mutex<TelemetryHistory>>,
    mouse_pos: Option<POINT>,
    is_mouse_tracking: bool,
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
            eprintln!("[ChartWindow] Lỗi khởi tạo cửa sổ biểu đồ nhiệt độ: {:?}", e);
        }
    });
}

/// Chạy vòng lặp thông điệp của cửa sổ biểu đồ nhiệt độ
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

        // Kích thước mặc định 800 x 520, căn giữa màn hình
        let width = 800;
        let height = 520;
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let pos_x = (screen_w - width) / 2;
        let pos_y = (screen_h - height) / 2;

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("Acer Predator - Lịch sử nhiệt độ phần cứng (1 Giờ)"),
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

/// Helper tạo HFONT Segoe UI
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

/// Vẽ toàn bộ nội dung biểu đồ lên Memory DC (Double-Buffering)
unsafe fn paint_chart(hdc: HDC, width: i32, height: i32, state: &ChartWindowState) {
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

    // Nạp dữ liệu thống kê và lịch sử từ bộ đệm Telemetry
    let (stats_opt, samples) = if let Ok(hist) = state.telemetry.lock() {
        (hist.stats(), hist.iter_chronological())
    } else {
        (None, Vec::new())
    };

    // 2. Vẽ Header Bar (Thông số thống kê thời gian thực)
    let header_font = create_chart_font(16, FW_SEMIBOLD.0 as i32);
    let old_font = SelectObject(hdc, header_font);

    match stats_opt {
        Some(stats) => {
            let cpu_line = format!(
                "[■ CPU] Hiện tại: {:2}°C | Min: {:2}°C | Max: {:2}°C | Trung bình: {:.1}°C",
                stats.cpu_current, stats.cpu_min, stats.cpu_max, stats.cpu_avg
            );
            draw_text_line(hdc, 20, 12, &cpu_line, COLOR_CPU);

            let gpu_line = format!(
                "[■ GPU] Hiện tại: {:2}°C | Min: {:2}°C | Max: {:2}°C | Trung bình: {:.1}°C",
                stats.gpu_current, stats.gpu_min, stats.gpu_max, stats.gpu_avg
            );
            draw_text_line(hdc, 20, 33, &gpu_line, COLOR_GPU);
        }
        None => {
            draw_text_line(hdc, 20, 12, "[■ CPU] Đang thu thập dữ liệu cảm biến...", COLOR_CPU);
            draw_text_line(hdc, 20, 33, "[■ GPU] Đang thu thập dữ liệu cảm biến...", COLOR_GPU);
        }
    }

    // 3. Xác định vùng đồ thị (Plot Area): Lề trái 60, lề trên 60, lề phải 25, lề dưới 40
    let plot_left = 60;
    let plot_top = 60;
    let plot_right = width - 25;
    let plot_bottom = height - 40;

    let plot_width = plot_right - plot_left;
    let plot_height = plot_bottom - plot_top;

    if plot_width <= 50 || plot_height <= 50 {
        let _ = SelectObject(hdc, old_font);
        let _ = DeleteObject(header_font);
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

    // Khởi tạo font chữ nhỏ cho trục tọa độ và nhãn
    let label_font = create_chart_font(13, FW_NORMAL.0 as i32);
    let _ = SelectObject(hdc, label_font);

    // 4. Trục Y: Nhiệt độ 30°C đến 100°C (bước 10°C)
    let grid_pen = CreatePen(PS_SOLID, 1, COLOR_GRID);
    let old_pen = SelectObject(hdc, grid_pen);

    for temp in (30..=100).step_by(10) {
        let ratio = (temp - 30) as f32 / 70.0;
        let y = plot_bottom - (ratio * plot_height as f32).round() as i32;

        // Đường lưới ngang
        let pts = [POINT { x: plot_left, y }, POINT { x: plot_right, y }];
        let _ = Polyline(hdc, &pts);

        // Nhãn nhiệt độ (°C) căn sát mép trái
        let label = format!("{:3}°C", temp);
        draw_text_line(hdc, plot_left - 48, y - 7, &label, COLOR_TEXT_MUTED);
    }

    // Đường cảnh báo 85°C (Nét đứt màu đỏ #E53935)
    let warn_y = plot_bottom - (((85.0 - 30.0) / 70.0) * plot_height as f32).round() as i32;
    let warn_pen = CreatePen(PS_DASH, 1, COLOR_WARN_85);
    let _ = SelectObject(hdc, warn_pen);
    let warn_pts = [POINT { x: plot_left, y: warn_y }, POINT { x: plot_right, y: warn_y }];
    let _ = Polyline(hdc, &warn_pts);
    draw_text_line(hdc, plot_right - 88, warn_y - 15, "85°C Warning", COLOR_WARN_85);
    let _ = DeleteObject(warn_pen);

    // 5. Trục X: Thời gian 1 giờ (3600 giây), các mốc: -60m, -50m, -40m, -30m, -20m, -10m, Now
    let _ = SelectObject(hdc, grid_pen);
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
        let tick_pts = [POINT { x, y: plot_bottom }, POINT { x, y: plot_bottom + 4 }];
        let _ = Polyline(hdc, &tick_pts);

        // Nhãn thời gian
        let offset_x = if mins_ago == 0 { -12 } else { -16 };
        draw_text_line(hdc, x + offset_x, plot_bottom + 8, mark_text, COLOR_TEXT_MUTED);
    }

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

    // 6. Vẽ các đường biểu diễn (Polyline)
    let mut mapped_samples: Vec<(i32, i32, i32, &TelemetrySample)> = Vec::with_capacity(samples.len());

    if samples.is_empty() {
        draw_text_line(
            hdc,
            plot_left + (plot_width / 2) - 130,
            plot_top + (plot_height / 2) - 8,
            "Chưa có dữ liệu cảm biến (đang thu thập...)",
            COLOR_TEXT_MUTED,
        );
    } else {
        let newest_ts = samples.last().map(|s| s.timestamp_secs).unwrap_or(0);

        let mut cpu_pts: Vec<POINT> = Vec::with_capacity(samples.len());
        let mut gpu_pts: Vec<POINT> = Vec::with_capacity(samples.len());

        for s in &samples {
            let k = newest_ts.saturating_sub(s.timestamp_secs);
            if k > 3600 {
                continue;
            }

            let x = plot_right - ((k as f32 / 3600.0) * plot_width as f32).round() as i32;
            let x_clamped = x.clamp(plot_left, plot_right);

            let cpu_ratio = (s.cpu_temp.clamp(30, 100) - 30) as f32 / 70.0;
            let cpu_y = plot_bottom - (cpu_ratio * plot_height as f32).round() as i32;

            let gpu_ratio = (s.gpu_temp.clamp(30, 100) - 30) as f32 / 70.0;
            let gpu_y = plot_bottom - (gpu_ratio * plot_height as f32).round() as i32;

            cpu_pts.push(POINT { x: x_clamped, y: cpu_y });
            gpu_pts.push(POINT { x: x_clamped, y: gpu_y });

            mapped_samples.push((x_clamped, cpu_y, gpu_y, s));
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

    // 7. Tương tác chuột (Crosshair & Tooltip)
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

                // Tính toán thông tin hiển thị Tooltip
                let newest_ts = samples.last().map(|s| s.timestamp_secs).unwrap_or(0);
                let k = newest_ts.saturating_sub(sample.timestamp_secs);
                let time_str = if k == 0 {
                    "Hiện tại (Now)".to_string()
                } else if k < 60 {
                    format!("-{} giây trước", k)
                } else {
                    format!("-{}m {:02}s trước", k / 60, k % 60)
                };

                // Vẽ hộp Tooltip
                let tt_w = 168;
                let tt_h = 76;
                let mut tt_x = closest_x + 15;
                let mut tt_y = m_pos.y - (tt_h / 2);

                if tt_x + tt_w > plot_right + 15 {
                    tt_x = closest_x - tt_w - 15;
                }
                if tt_y < plot_top + 5 {
                    tt_y = plot_top + 5;
                }
                if tt_y + tt_h > plot_bottom - 5 {
                    tt_y = plot_bottom - tt_h - 5;
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

                draw_text_line(hdc, tt_x + 10, tt_y + 8, &time_str, COLOR_TEXT_LIGHT);
                draw_text_line(
                    hdc,
                    tt_x + 10,
                    tt_y + 28,
                    &format!("CPU: {}°C (Quạt: {}%)", sample.cpu_temp, sample.cpu_fan_pct),
                    COLOR_CPU,
                );
                draw_text_line(
                    hdc,
                    tt_x + 10,
                    tt_y + 48,
                    &format!("GPU: {}°C (Quạt: {}%)", sample.gpu_temp, sample.gpu_fan_pct),
                    COLOR_GPU,
                );
            }
        }
    }

    // Dọn dẹp tài nguyên GDI
    let _ = SelectObject(hdc, old_pen);
    let _ = DeleteObject(grid_pen);

    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(label_font);
    let _ = DeleteObject(header_font);
}

/// Window Procedure cho cửa sổ biểu đồ nhiệt độ
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

                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ChartWindowState;
                if let Some(state) = ptr.as_ref() {
                    paint_chart(hdc_mem, width, height, state);
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
            // Giới hạn kích thước tối thiểu khi người dùng co dãn cửa sổ
            let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
            mmi.ptMinTrackSize.x = 550;
            mmi.ptMinTrackSize.y = 360;
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
    fn test_x_coordinate_mapping() {
        let plot_left = 60;
        let plot_right = 775;
        let plot_width = plot_right - plot_left; // 715

        // Mẫu mới nhất k = 0
        let k_0 = 0f32;
        let x_0 = plot_right - ((k_0 / 3600.0) * plot_width as f32).round() as i32;
        assert_eq!(x_0, plot_right);

        // Mẫu cách 1 giờ k = 3600
        let k_3600 = 3600f32;
        let x_3600 = plot_right - ((k_3600 / 3600.0) * plot_width as f32).round() as i32;
        assert_eq!(x_3600, plot_left);

        // Mẫu cách 30 phút k = 1800
        let k_1800 = 1800f32;
        let x_1800 = plot_right - ((k_1800 / 3600.0) * plot_width as f32).round() as i32;
        let expected_mid = plot_left + (plot_width / 2);
        assert!((x_1800 - expected_mid).abs() <= 1);
    }

    #[test]
    fn test_y_coordinate_mapping() {
        let plot_top = 60;
        let plot_bottom = 480;
        let plot_height = plot_bottom - plot_top; // 420

        // 30°C -> ở đáy plot (plot_bottom)
        let t_30 = 30u8;
        let ratio_30 = (t_30.clamp(30, 100) - 30) as f32 / 70.0;
        let y_30 = plot_bottom - (ratio_30 * plot_height as f32).round() as i32;
        assert_eq!(y_30, plot_bottom);

        // 100°C -> ở đỉnh plot (plot_top)
        let t_100 = 100u8;
        let ratio_100 = (t_100.clamp(30, 100) - 30) as f32 / 70.0;
        let y_100 = plot_bottom - (ratio_100 * plot_height as f32).round() as i32;
        assert_eq!(y_100, plot_top);

        // 65°C -> chính giữa trục Y
        let t_65 = 65u8;
        let ratio_65 = (t_65.clamp(30, 100) - 30) as f32 / 70.0;
        let y_65 = plot_bottom - (ratio_65 * plot_height as f32).round() as i32;
        assert_eq!(y_65, (plot_bottom + plot_top) / 2);

        // Dưới 30°C -> clamp về đáy
        let t_below = 20u8;
        let ratio_below = (t_below.clamp(30, 100) - 30) as f32 / 70.0;
        let y_below = plot_bottom - (ratio_below * plot_height as f32).round() as i32;
        assert_eq!(y_below, plot_bottom);

        // Trên 100°C -> clamp về đỉnh
        let t_above = 115u8;
        let ratio_above = (t_above.clamp(30, 100) - 30) as f32 / 70.0;
        let y_above = plot_bottom - (ratio_above * plot_height as f32).round() as i32;
        assert_eq!(y_above, plot_top);
    }

    #[test]
    fn test_chart_window_state_telemetry_interaction() {
        let telemetry = Arc::new(Mutex::new(TelemetryHistory::new()));
        let state = ChartWindowState {
            telemetry: telemetry.clone(),
            mouse_pos: None,
            is_mouse_tracking: false,
        };

        // Khi rỗng
        {
            let hist = state.telemetry.lock().unwrap();
            assert!(hist.is_empty());
            assert_eq!(hist.stats(), None);
        }

        // Sau khi đẩy dữ liệu
        {
            let mut hist = telemetry.lock().unwrap();
            hist.push_with_timestamp(100, 48, 42, 35, 30);
            hist.push_with_timestamp(101, 52, 45, 40, 35);
        }

        {
            let hist = state.telemetry.lock().unwrap();
            assert_eq!(hist.len(), 2);
            let stats = hist.stats().unwrap();
            assert_eq!(stats.cpu_current, 52);
            assert_eq!(stats.cpu_min, 48);
            assert_eq!(stats.cpu_max, 52);
            assert_eq!(stats.gpu_current, 45);
        }
    }
}
