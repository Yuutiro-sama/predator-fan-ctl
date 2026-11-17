#![windows_subsystem = "windows"]

//! Acer Predator Ultra-Light Fan Controller (Rust Pure Win32 Native)
//!
//! Entry point & Orchestration layer:
//! - Tích hợp toàn diện các module: `scm`, `sensor`, `fan_backend`, `curve`, `config`, `tray`.
//! - Tự động gắn kết Console nếu chạy từ PowerShell/CMD (AttachConsole) và chạy ngầm 100% không hiện console nếu click chạy trực tiếp.
//! - Cơ chế Graceful Shutdown đa tầng (SetConsoleCtrlHandler, WM_CLOSE, WM_DESTROY, Tray Exit).
//! - Hardware Fail-Safe: Luôn khôi phục quạt về chế độ Tự động (BIOS Auto) và trả lại PredatorSenseService trước khi thoát.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, SetLastError, BOOL, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM,
    WIN32_ERROR, WPARAM,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Console::{AttachConsole, SetConsoleCtrlHandler, ATTACH_PARENT_PROCESS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PostMessageW, SetForegroundWindow, WM_CLOSE, WM_COMMAND,
};

use predator_fan_ctl::config::Config;
use predator_fan_ctl::curve::evaluate_curve;
use predator_fan_ctl::fan_backend::{
    AcerWmiBackend, DirectEcBackend, EcRegisterMap, FanController,
};
use predator_fan_ctl::governor::ThermalGovernor;
use predator_fan_ctl::scm::ServiceGuard;
use predator_fan_ctl::sensor::SystemTelemetryReader;
use predator_fan_ctl::tray::{FanProfile, TrayCommand, TrayManager};

/// Cờ báo hiệu yêu cầu dừng toàn hệ thống
static GLOBAL_RUNNING: AtomicBool = AtomicBool::new(true);

/// Lưu trữ HWND của cửa sổ khay hệ thống để nhận tín hiệu ngắt từ Console Handler
static GLOBAL_TRAY_HWND: AtomicIsize = AtomicIsize::new(0);

/// Fallback Dummy Backend khi chạy trên môi trường không có phần cứng Acer hoặc chưa có driver EC
struct FallbackBackend;

impl FanController for FallbackBackend {
    fn set_manual_mode(&mut self) -> bool {
        true
    }

    fn set_auto_mode(&mut self) -> bool {
        true
    }

    fn set_speeds(&mut self, _cpu_pct: u8, _gpu_pct: u8) -> bool {
        true
    }

    fn read_temps(&mut self) -> Option<(u32, u32)> {
        None
    }

    fn name(&self) -> &'static str {
        "Fallback Backend (Simulation / Safe Default)"
    }
}

/// Handler xử lý tín hiệu ngắt Console (Ctrl+C, Ctrl+Break, Đóng cửa sổ CMD)
unsafe extern "system" fn console_ctrl_handler(ctrl_type: u32) -> BOOL {
    match ctrl_type {
        0..=2 /* CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT */ => {
            println!("\n[SHUTDOWN] Received console interrupt signal ({ctrl_type}). Triggering graceful shutdown...");
            GLOBAL_RUNNING.store(false, Ordering::SeqCst);
            let raw_hwnd = GLOBAL_TRAY_HWND.load(Ordering::Relaxed);
            if raw_hwnd != 0 {
                let _ = PostMessageW(HWND(raw_hwnd as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            BOOL(1)
        }
        _ => BOOL(0),
    }
}

/// Khởi tạo backend điều khiển quạt phần cứng thích hợp dựa theo cấu hình
fn init_fan_backend(config: &Config) -> Box<dyn FanController> {
    if config.backend.eq_ignore_ascii_case("ec") {
        let regs = EcRegisterMap {
            cpu_fan_mode_reg: config.ec_registers.cpu_fan_mode_reg,
            cpu_fan_speed_reg: config.ec_registers.cpu_fan_speed_reg,
            gpu_fan_mode_reg: config.ec_registers.gpu_fan_mode_reg,
            gpu_fan_speed_reg: config.ec_registers.gpu_fan_speed_reg,
            cpu_temp_reg: 0xB0,
            gpu_temp_reg: 0xB1,
        };
        if let Some(ec) = DirectEcBackend::new(regs) {
            println!("[BACKEND] Initialized Direct EC Backend (Port 0x62/0x66)");
            return Box::new(ec);
        }
        eprintln!("[BACKEND WARN] Direct EC init failed (missing driver). Falling back to Acer WMI...");
    }

    if let Some(wmi) = AcerWmiBackend::new() {
        println!("[BACKEND] Initialized Acer WMI Backend (root\\WMI -> AcerGamingFunction)");
        return Box::new(wmi);
    }

    // Dự phòng Direct EC nếu WMI không khả dụng
    let regs = EcRegisterMap {
        cpu_fan_mode_reg: config.ec_registers.cpu_fan_mode_reg,
        cpu_fan_speed_reg: config.ec_registers.cpu_fan_speed_reg,
        gpu_fan_mode_reg: config.ec_registers.gpu_fan_mode_reg,
        gpu_fan_speed_reg: config.ec_registers.gpu_fan_speed_reg,
        cpu_temp_reg: 0xB0,
        gpu_temp_reg: 0xB1,
    };
    if let Some(ec) = DirectEcBackend::new(regs) {
        println!("[BACKEND] Initialized Direct EC Backend as fallback");
        return Box::new(ec);
    }

    eprintln!("[BACKEND WARN] No hardware fan controller available. Operating in Fallback Safe Mode.");
    Box::new(FallbackBackend)
}

/// Bộ kiểm soát đơn phiên bản (Single-Instance Guard) bằng Win32 Named Mutex
pub struct SingleInstanceGuard {
    handle: HANDLE,
    already_running: bool,
}

impl SingleInstanceGuard {
    pub fn acquire(name: &str) -> windows::core::Result<Self> {
        let wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let (handle, already_running) = unsafe {
            SetLastError(WIN32_ERROR(0));
            match CreateMutexW(None, true, PCWSTR(wide_name.as_ptr())) {
                Ok(h) => {
                    let exists = GetLastError() == ERROR_ALREADY_EXISTS;
                    (h, exists)
                }
                Err(_) => {
                    let last_err = GetLastError();
                    // ERROR_ACCESS_DENIED (5) chứng minh Mutex Global đã tồn tại do tiến trình quyền cao tạo
                    if last_err.0 == 5 {
                        (HANDLE::default(), true)
                    } else {
                        let local_name = format!("Local\\{}", name.strip_prefix("Global\\").unwrap_or(name));
                        let wide_local: Vec<u16> = local_name.encode_utf16().chain(std::iter::once(0)).collect();
                        SetLastError(WIN32_ERROR(0));
                        match CreateMutexW(None, true, PCWSTR(wide_local.as_ptr())) {
                            Ok(h) => {
                                let exists = GetLastError() == ERROR_ALREADY_EXISTS;
                                (h, exists)
                            }
                            Err(_) => (HANDLE::default(), GetLastError().0 == 5),
                        }
                    }
                }
            }
        };

        Ok(Self {
            handle,
            already_running,
        })
    }

    #[inline]
    pub fn is_already_running(&self) -> bool {
        self.already_running
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

fn main() {
    // 0. Kích hoạt Per-Monitor DPI Awareness v2 để chống mờ nhòe font chữ trên Windows
    unsafe {
        use windows::Win32::UI::HiDpi::*;
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // 1. Tự động kết nối tới Console của tiến trình cha (nếu được khởi chạy từ PowerShell/CMD)
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }

    if std::env::args().any(|a| a == "--curve-standalone") {
        let _ = predator_fan_ctl::curve_dialog::show_dialog_standalone();
        return;
    }

    println!("==================================================================");
    println!("  Acer Predator Ultra-Light Fan Controller (Rust Win32 Native)   ");
    println!("==================================================================");

    // 2. Kiểm tra đơn phiên bản (Single-Instance Check)
    // Ngăn chặn hoàn toàn việc chạy 2 hoặc nhiều app cùng lúc làm lặp tray icon và tranh chấp thanh ghi
    let find_tray = || -> Option<HWND> {
        unsafe {
            if let Ok(h) = FindWindowW(w!("PredatorFanTrayWindowClass"), w!("PredatorFanTray")) {
                if !h.0.is_null() {
                    return Some(h);
                }
            }
            if let Ok(h) = FindWindowW(PCWSTR::null(), w!("PredatorFanTray")) {
                if !h.0.is_null() {
                    return Some(h);
                }
            }
        }
        None
    };

    let cmd = if std::env::args().any(|a| a == "--chart" || a == "-c") {
        predator_fan_ctl::tray::IDM_SHOW_TEMP_CHART
    } else if std::env::args().any(|a| a == "--curve") {
        predator_fan_ctl::tray::IDM_CUSTOMIZE_CURVE
    } else {
        predator_fan_ctl::tray::IDM_OPEN_DASHBOARD
    };

    let _single_instance_guard = match SingleInstanceGuard::acquire("Global\\AcerPredatorFanControlMutex") {
        Ok(guard) => {
            if guard.is_already_running() {
                println!("[SINGLE-INSTANCE] Một phiên bản khác của Acer Predator Fan Control đã đang chạy!");
                for _ in 0..10 {
                    if let Some(hwnd) = find_tray() {
                        unsafe {
                            let _ = PostMessageW(hwnd, WM_COMMAND, WPARAM(cmd), LPARAM(0));
                            let _ = SetForegroundWindow(hwnd);
                        }
                        println!("[SINGLE-INSTANCE] Đã gửi lệnh kích hoạt giao diện (cmd: {cmd}) tới app đang chạy.");
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                println!("[SINGLE-INSTANCE] Không tìm thấy cửa sổ khay để chuyển giao lệnh.");
                return;
            }
            guard
        }
        Err(e) => {
            eprintln!("[WARN] Không thể tạo mutex đơn phiên bản ({e}). Tiếp tục khởi động...");
            SingleInstanceGuard {
                handle: HANDLE::default(),
                already_running: false,
            }
        }
    };

    // 3. Đăng ký Win32 Console Control Handler để xử lý Ctrl+C an toàn
    unsafe {
        if SetConsoleCtrlHandler(Some(console_ctrl_handler), true).is_err() {
            eprintln!("[WARN] Failed to register console control handler");
        }
    }

    // 4. Nạp cấu hình từ config.toml (hoặc tự tạo cấu hình mặc định an toàn)
    let config_path = predator_fan_ctl::config::get_config_path();
    let config = Config::load_or_default(&config_path);
    println!("[CONFIG] Polling interval: {}ms, Backend: '{}', Path: '{}'",
        config.poll_interval_ms, config.backend, config_path.display());

    // 5. Quản lý Windows Service OEM qua Win32 SCM API (tương thích đa thế hệ PredatorSense 3.0 - 5.0)
    let candidate_services = [
        config.service_name.as_str(),
        "AASSvc",               // Acer Agent Service (PredatorSense 5.0)
        "ASMSvc",               // Acer System Monitor Service
        "PredatorSenseService", // Acer PredatorSense 3.0/4.0
    ];
    let service_guard = ServiceGuard::acquire_many(&candidate_services, config.manage_predator_service);
    let stopped = service_guard.stopped_services();
    if !stopped.is_empty() {
        println!("[SCM] Đã tạm dừng các service OEM để chiếm quyền độc quyền EC: {}", stopped.join(", "));
    } else if service_guard.is_managed() {
        println!("[SCM] Không phát hiện service OEM đang chạy nền xung đột.");
    }

    // 5. Khởi tạo bộ đệm Telemetry (Lưu trữ 1 giờ lịch sử nhiệt độ & tốc độ quạt)
    let telemetry = Arc::new(Mutex::new(predator_fan_ctl::telemetry::TelemetryHistory::new()));

    // 6. Khởi tạo Win32 Native System Tray Icon
    let (tray_manager, tray_handle, cmd_receiver) = match TrayManager::create(telemetry.clone()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[FATAL] Failed to create System Tray Icon: {e}");
            service_guard.restore();
            return;
        }
    };

    // Lưu HWND vào AtomicIsize để Console Handler có thể kích hoạt WM_CLOSE
    GLOBAL_TRAY_HWND.store(tray_manager.hwnd().0 as isize, Ordering::SeqCst);
    println!("[TRAY] System tray icon registered successfully. Running in background.");

    // 7. Khởi chạy luồng Worker điều khiển quạt chu kỳ (Controller Thread)
    let worker_running = Arc::new(AtomicBool::new(true));
    let worker_running_clone = worker_running.clone();
    let worker_tray_handle = tray_handle.clone();
    let worker_telemetry = telemetry.clone();
    let mut worker_config = config.clone();

    let worker_handle = thread::spawn(move || {
        // Khởi tạo COM Apartment cho riêng Worker Thread
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        // Khởi tạo Backend phần cứng bên trong Worker Thread
        let mut backend = init_fan_backend(&worker_config);
        println!("[CONTROLLER] Active Backend: {}", backend.name());
        backend.set_manual_mode();

        // Khởi tạo Trình đọc telemetry toàn diện (NVML FFI cho GPU, ACPI cho CPU Temp, PowrProf cho CPU Clock, PDH cho CPU Power)
        let mut telemetry_reader = SystemTelemetryReader::new();
        let (init_cpu_t, init_gpu_t, init_cpu_power, init_gpu_power, init_cpu_mhz, init_gpu_mhz) = telemetry_reader.poll();
        println!("[SENSOR] Initial telemetry - CPU: {}°C ({:.1}W, {}MHz), GPU: {}°C ({:.1}W, {}MHz) (NVML GPU sensor active: {})",
            init_cpu_t, init_cpu_power as f32 / 10.0, init_cpu_mhz,
            init_gpu_t, init_gpu_power as f32 / 10.0, init_gpu_mhz,
            telemetry_reader.has_gpu_sensor());

        let mut last_cpu_speed = evaluate_curve(&worker_config.cpu_curve, init_cpu_t);
        let mut last_gpu_speed = evaluate_curve(&worker_config.gpu_curve, init_gpu_t);
        backend.set_speeds(last_cpu_speed, last_gpu_speed);
        let (init_cpu_rpm, init_gpu_rpm) = backend.read_fan_rpms().unwrap_or((0, 0));
        worker_tray_handle.update_full_status(
            init_cpu_t, last_cpu_speed, init_gpu_t, last_gpu_speed, init_cpu_rpm, init_gpu_rpm,
            init_cpu_power, init_gpu_power, init_cpu_mhz, init_gpu_mhz,
        );
        let init_tip = format!(
            "CPU: {}°C {:.1}W {:.2}GHz | GPU: {}°C {:.1}W {:.2}GHz",
            init_cpu_t,
            init_cpu_power as f32 / 10.0,
            init_cpu_mhz as f32 / 1000.0,
            init_gpu_t,
            init_gpu_power as f32 / 10.0,
            init_gpu_mhz as f32 / 1000.0,
        );
        worker_tray_handle.update_tooltip(&init_tip);
        if let Ok(mut hist) = worker_telemetry.try_lock() {
            hist.push(
                init_cpu_t as u8, init_gpu_t as u8, last_cpu_speed, last_gpu_speed,
                init_cpu_power, init_gpu_power, init_cpu_mhz, init_gpu_mhz,
            );
        }

        // Khởi tạo Smooth EPP Thermal Governor (Intel Speed Shift - Zero Stuttering)
        let mut governor = ThermalGovernor::new((&worker_config.thermal_governor).into());
        worker_tray_handle.update_governor_status(
            governor.config.enabled,
            governor.config.target_temp,
            governor.current_epp,
        );
        println!("[GOVERNOR] Initialized Smooth EPP Thermal Governor (Target: {}°C, Max EPP: {}, Enabled: {})",
            governor.config.target_temp, governor.config.max_epp, governor.config.enabled);

        let mut last_watchdog_refresh = Instant::now();

        while worker_running_clone.load(Ordering::Relaxed) && GLOBAL_RUNNING.load(Ordering::Relaxed) {
            // Xử lý các lệnh từ menu khay hệ thống (non-blocking)
            while let Ok(cmd) = cmd_receiver.try_recv() {
                match cmd {
                    TrayCommand::SetProfile(profile) => {
                        worker_tray_handle.set_profile(profile);
                        match profile {
                            FanProfile::ActiveCurve => {
                                backend.set_manual_mode();
                                last_cpu_speed = 255; // Ép cập nhật lại theo đường cong
                                last_gpu_speed = 255;
                                println!("[MODE] Switched to Active Fan Curve");
                            }
                            FanProfile::Turbo => {
                                backend.set_manual_mode();
                                backend.set_speeds(100, 100);
                                last_cpu_speed = 100;
                                last_gpu_speed = 100;
                                println!("[MODE] Switched to Turbo Mode (100% Speed)");
                            }
                            FanProfile::Silent => {
                                backend.set_manual_mode();
                                backend.set_speeds(25, 25);
                                last_cpu_speed = 25;
                                last_gpu_speed = 25;
                                println!("[MODE] Switched to Silent Mode (25% Speed)");
                            }
                        }
                    }
                    TrayCommand::ReloadConfig => {
                        let config_path = predator_fan_ctl::config::get_config_path();
                        let reloaded = Config::load_or_default(&config_path);
                        println!("[CONFIG] Hot-reloaded configuration from '{}' successfully (poll: {}ms)",
                            config_path.display(), reloaded.poll_interval_ms);
                        governor.config = (&reloaded.thermal_governor).into();
                        worker_tray_handle.update_governor_status(
                            governor.config.enabled,
                            governor.config.target_temp,
                            governor.current_epp,
                        );
                        worker_config = reloaded;
                        last_cpu_speed = 255;
                        last_gpu_speed = 255;
                    }
                    TrayCommand::OpenCurveDialog => {
                        // Cửa sổ cấu hình được mở độc lập trên UI/Worker thread
                    }
                    TrayCommand::ShowTempChart => {
                        // Cửa sổ biểu đồ được mở độc lập trên UI thread
                    }
                    TrayCommand::ToggleGovernor => {
                        governor.config.enabled = !governor.config.enabled;
                        if !governor.config.enabled {
                            governor.reset_to_maximum();
                        }
                        worker_tray_handle.update_governor_status(
                            governor.config.enabled,
                            governor.config.target_temp,
                            governor.current_epp,
                        );
                        println!("[GOVERNOR] Smooth EPP Thermal Governor toggled to: {}", if governor.config.enabled { "ENABLED" } else { "DISABLED" });
                    }
                    TrayCommand::Exit => {
                        println!("[COMMAND] Exit requested from System Tray menu.");
                        worker_running_clone.store(false, Ordering::SeqCst);
                        GLOBAL_RUNNING.store(false, Ordering::SeqCst);
                        let raw_hwnd = GLOBAL_TRAY_HWND.load(Ordering::Relaxed);
                        if raw_hwnd != 0 {
                            unsafe {
                                let _ = PostMessageW(HWND(raw_hwnd as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
                            }
                        }
                        break;
                    }
                }
            }

            if !worker_running_clone.load(Ordering::Relaxed) || !GLOBAL_RUNNING.load(Ordering::Relaxed) {
                break;
            }

            // Đọc toàn bộ thông số cảm biến thời gian thực
            let (cpu_temp, gpu_temp, cpu_power, gpu_power, cpu_mhz, gpu_mhz) = telemetry_reader.poll();

            // Tính toán tốc độ quạt mục tiêu dựa theo Profile
            let active_profile = worker_tray_handle.get_profile();
            let (target_cpu_speed, target_gpu_speed) = match active_profile {
                FanProfile::ActiveCurve => {
                    let cs = evaluate_curve(&worker_config.cpu_curve, cpu_temp);
                    let gs = evaluate_curve(&worker_config.gpu_curve, gpu_temp);
                    (cs, gs)
                }
                FanProfile::Turbo => (100, 100),
                FanProfile::Silent => (25, 25),
            };

            // Cơ chế Fail-Safe & Tối ưu: Chỉ ghi bus phần cứng khi tốc độ thay đổi
            // hoặc làm mới định kỳ mỗi 10 giây để chống Watchdog BIOS tự reset
            let need_hw_write = target_cpu_speed != last_cpu_speed
                || target_gpu_speed != last_gpu_speed
                || last_watchdog_refresh.elapsed() >= Duration::from_secs(10);

            if need_hw_write {
                backend.set_speeds(target_cpu_speed, target_gpu_speed);
                last_cpu_speed = target_cpu_speed;
                last_gpu_speed = target_gpu_speed;
                last_watchdog_refresh = Instant::now();
            }

            // Cập nhật Smooth EPP Thermal Governor (Zero Stuttering)
            let current_epp = governor.update(cpu_temp);
            worker_tray_handle.update_governor_status(
                governor.config.enabled,
                governor.config.target_temp,
                current_epp,
            );

            // Đọc RPM thời gian thực từ cảm biến phần cứng
            let (cpu_rpm, gpu_rpm) = backend.read_fan_rpms().unwrap_or((0, 0));

            // Cập nhật trạng thái System Tray (bao gồm cả công suất và xung nhịp)
            worker_tray_handle.update_full_status(
                cpu_temp, target_cpu_speed, gpu_temp, target_gpu_speed, cpu_rpm, gpu_rpm,
                cpu_power, gpu_power, cpu_mhz, gpu_mhz,
            );

            // Cập nhật bộ đệm lịch sử Telemetry (Zero-Allocation micro-lock)
            if let Ok(mut hist) = worker_telemetry.try_lock() {
                hist.push(
                    cpu_temp as u8, gpu_temp as u8, target_cpu_speed, target_gpu_speed,
                    cpu_power, gpu_power, cpu_mhz, gpu_mhz,
                );
            }

            // Cập nhật tooltip khay hệ thống hiển thị: "CPU: XX°C XX.XW X.XXGHz | GPU: XX°C XX.XW X.XXGHz"
            let tip = format!(
                "CPU: {}°C {:.1}W {:.2}GHz | GPU: {}°C {:.1}W {:.2}GHz",
                cpu_temp,
                cpu_power as f32 / 10.0,
                cpu_mhz as f32 / 1000.0,
                gpu_temp,
                gpu_power as f32 / 10.0,
                gpu_mhz as f32 / 1000.0,
            );
            worker_tray_handle.update_tooltip(&tip);

            // Ngủ theo khoảng thời gian cấu hình (chia nhỏ 50ms để đáp ứng lệnh thoát tức thì)
            let poll_ms = worker_config.poll_interval_ms.max(100);
            let chunks = (poll_ms / 50).max(1);
            for _ in 0..chunks {
                if !worker_running_clone.load(Ordering::Relaxed) || !GLOBAL_RUNNING.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }

        // 7. Hardware Fail-Safe: Khôi phục quạt về chế độ Tự động (BIOS Auto) và phục hồi EPP = 0, PROCTHROTTLEMAX = 100% trước khi thoát luồng
        println!("[FAIL-SAFE] Reverting Thermal Governor EPP to 0 (Max Performance) and PROCTHROTTLEMAX to 100%...");
        governor.reset_to_maximum();
        println!("[FAIL-SAFE] Setting hardware fan controller to BIOS Auto mode...");
        backend.set_auto_mode();
        println!("[FAIL-SAFE] Fans successfully reverted to BIOS default.");
    });

    // 8. Nếu được gọi với cờ --chart hoặc --dashboard
    if std::env::args().any(|a| a == "--chart" || a == "-c") {
        predator_fan_ctl::chart_window::show_or_focus_chart_window(telemetry.clone());
    } else if std::env::args().any(|a| a == "--dashboard" || a == "-d") {
        predator_fan_ctl::dashboard_window::show_or_focus_dashboard(
            tray_manager.state(),
            tray_manager.cmd_sender(),
            telemetry.clone(),
        );
    }

    // 9. Chạy Win32 Native Message Loop trên Main Thread (GetMessageW - 0% CPU khi không tương tác)
    let _ = tray_manager.run_message_loop();

    // 9. Dọn dẹp an toàn khi vòng lặp thông điệp kết thúc
    GLOBAL_RUNNING.store(false, Ordering::SeqCst);
    worker_running.store(false, Ordering::SeqCst);

    if let Err(e) = worker_handle.join() {
        eprintln!("[WARN] Worker thread join error: {:?}", e);
    }

    // 10. Khôi phục dịch vụ OEM Windows (PredatorSenseService)
    drop(service_guard);
    println!("[CLEANUP] OEM Service restored. Shutdown complete.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_instance_guard_lifecycle() {
        let test_mutex_name = "Local\\PredatorFanTestUniqueMutex_12345";
        let guard1 = SingleInstanceGuard::acquire(test_mutex_name)
            .expect("First mutex acquire must succeed");
        assert!(!guard1.is_already_running(), "First instance must report not already running");

        // Khi thử tạo instance thứ 2 với cùng tên mutex
        let guard2 = SingleInstanceGuard::acquire(test_mutex_name)
            .expect("Second mutex acquire must return handle");
        assert!(guard2.is_already_running(), "Second instance MUST detect running instance");

        // Sau khi thả instance 2, instance 1 vẫn đang giữ
        drop(guard2);

        // Thả instance 1
        drop(guard1);

        // Instance mới sau khi instance cũ đã thoát sẽ không bị coi là already running
        let guard3 = SingleInstanceGuard::acquire(test_mutex_name)
            .expect("Third mutex acquire must succeed");
        assert!(!guard3.is_already_running(), "New instance after release must not report already running");
    }
}
