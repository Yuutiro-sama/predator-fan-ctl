KẾ HOẠCH TRIỂN KHAI TỔNG THỂ (MASTER IMPLEMENTATION PLAN): ACER PREDATOR ULTRA-LIGHT FAN CONTROLLER (RUST WIN32 NATIVE)

Kế hoạch này định nghĩa kiến trúc và lộ trình thực thi từng bước để kỹ sư Antigravity xây dựng ứng dụng điều khiển quạt Acer Predator siêu nhẹ bằng Rust thuần (Pure Win32 API), loại bỏ hoàn toàn các UI framework nặng nề, đảm bảo mức tiêu thụ tài nguyên thực tế RAM < 2.5 MB (vượt xa ngưỡng yêu cầu < 15 MB) và CPU < 0.05%.

1. Kiến trúc Hệ thống & Định mức Tài nguyên
Thành phần Kiến trúc	Giải pháp Kỹ thuật (Rust Native)	Chỉ tiêu Kỹ thuật
Ngôn ngữ & ABI	Rust (x86_64-pc-windows-msvc), #![windows_subsystem = "windows"]	Binary < 1.5 MB
Quản lý Dịch vụ	Win32 SCM (OpenSCManagerW, ControlService, StartServiceW)	Tự động dừng/khôi phục PredatorSenseService
Điều khiển Quạt (Chính)	COM/WMI (root\WMI -> AcerGaming_SystemInformation)	Không cần driver bên thứ 3
Điều khiển Quạt (Dự phòng)	Direct EC Port I/O (0x62/0x66) qua Kernel Driver (inpoutx64.sys)	Độ trễ < 1ms
Đọc Nhiệt độ (Sensor)	WMI ACPI Thermal / NVAPI (nvml.dll dynamic FFI)	Chu kỳ cấu hình (mặc định 1000ms)
Thuật toán Fan Curve	Nội suy tuyến tính từng đoạn (Piecewise Linear), 0-Hysteresis	Phản ứng tức thì theo xung nhịp đọc
Giao diện (UI)	Win32 Shell_NotifyIconW (System Tray) + Popup Menu (CreatePopupMenu)	0 MB GUI overhead khi chạy ngầm
2. Cấu trúc Thư mục Dự án (Project Layout)

Kỹ sư Antigravity khởi tạo cấu trúc thư mục tại workspace như sau:

Plaintext
predator-fan-ctl/
├── Cargo.toml
├── build.rs
├── app.manifest
├── config.toml
└── src/
    ├── main.rs          # Entry point, Win32 Message Loop, Graceful Shutdown Hook
    ├── config.rs        # Parser config.toml & Hot-reload
    ├── scm.rs           # Quản lý bật/tắt PredatorSenseService
    ├── sensor.rs        # Đọc nhiệt độ CPU/GPU (WMI & NVML FFI)
    ├── fan_backend.rs   # WMI AcerGaming_SystemInformation & Direct EC Register I/O
    ├── curve.rs         # Toán học Fan Curve nội suy tuyến tính không trễ
    └── tray.rs          # Win32 System Tray Icon & Context Menu

3. Lộ trình Triển khai Chi tiết (Step-by-Step Task Breakdown)
Giai đoạn 1: Khởi tạo Dự án, Tối ưu Hóa Binary & Nhúng Manifest requireAdministrator

Bước 1.1: Cấu hình Cargo.toml
Tạo file Cargo.toml sử dụng crate windows chính chủ của Microsoft và cấu hình profile release tối ưu hóa kích thước/bộ nhớ:

Ini, TOML
[package]
name = "predator-fan-ctl"
version = "1.0.0"
edition = "2021"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
toml = "0.8"
wmi = "0.13"

[dependencies.windows]
version = "0.58"
features = [
    "Win32_Foundation",
    "Win32_System_Services",
    "Win32_System_Com",
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Graphics_Gdi",
    "Win32_System_LibraryLoader",
    "Win32_System_Threading",
]

[build-dependencies]
embed-resource = "2.4"

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true


Bước 1.2: Tạo app.manifest và build.rs
Đảm bảo ứng dụng luôn yêu cầu quyền Administrator để truy cập SCM, root\WMI và EC I/O port.

File app.manifest:

XML
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>


File build.rs:

Rust
fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    embed_resource::compile_for("app.manifest", ["predator-fan-ctl"], embed_resource::NONE);
}


(Lưu ý cho Antigravity: Tạo thêm file app.rc chứa dòng 1 24 "app.manifest" và truyền "app.rc" vào embed_resource::compile("app.rc", embed_resource::NONE) để trình biên dịch MSVC RC đóng gói manifest trực tiếp vào PE header).

Giai đoạn 2: Thiết kế Cấu hình (config.toml & src/config.rs)

Bước 2.1: Định nghĩa file config.toml

Ini, TOML
poll_interval_ms = 1000
backend = "wmi" # "wmi" hoặc "ec"
manage_predator_service = true
service_name = "PredatorSenseService"

# Cấu hình địa chỉ thanh ghi EC cho Acer Predator (sử dụng khi backend = "ec")
[ec_registers]
cpu_fan_mode_reg = 0x21
cpu_fan_speed_reg = 0x37
gpu_fan_mode_reg = 0x22
gpu_fan_speed_reg = 0x3A

# Fan curve phản ứng tức thì (Zero-Hysteresis): [Nhiệt độ (°C), Tốc độ quạt (%)]
[[cpu_curve]]
temp = 40
speed = 20

[[cpu_curve]]
temp = 60
speed = 50

[[cpu_curve]]
temp = 75
speed = 80

[[cpu_curve]]
temp = 85
speed = 100

[[gpu_curve]]
temp = 45
speed = 25

[[gpu_curve]]
temp = 65
speed = 60

[[gpu_curve]]
temp = 80
speed = 100


Bước 2.2: Triển khai src/config.rs và src/curve.rs (Zero-Hysteresis Engine)
Không sử dụng bộ đệm trễ (hysteresis buffer). Ngay khi nhiệt độ thay đổi 1°C trong chu kỳ polling, tốc độ quạt mới được tính toán nội suy tuyến tính tức thì:

Rust
// src/curve.rs
use serde::Deserialize;

#[derive(Deserialize, Clone, Copy)]
pub struct CurvePoint {
    pub temp: u32,
    pub speed: u8,
}

pub fn evaluate_curve(points: &[CurvePoint], current_temp: u32) -> u8 {
    if points.is_empty() {
        return 50;
    }
    if current_temp <= points[0].temp {
        return points[0].speed.min(100);
    }
    let last = &points[points.len() - 1];
    if current_temp >= last.temp {
        return last.speed.min(100);
    }

    for window in points.windows(2) {
        let p1 = window[0];
        let p2 = window[1];
        if current_temp >= p1.temp && current_temp <= p2.temp {
            if p2.temp == p1.temp {
                return p2.speed.min(100);
            }
            let ratio = (current_temp - p1.temp) as f32 / (p2.temp - p1.temp) as f32;
            let interpolated = p1.speed as f32 + ratio * (p2.speed as f32 - p1.speed as f32);
            return (interpolated.round() as u8).min(100);
        }
    }
    100
}

Giai đoạn 3: Quản lý PredatorSenseService qua Windows SCM API (src/scm.rs)

Khi ứng dụng khởi chạy, nó phải kiểm tra xem PredatorSenseService (hoặc AcerPredatorSenseService tùy đời máy) có đang chạy không. Nếu đang chạy, ứng dụng dừng dịch vụ này để tránh xung đột ghi đè thanh ghi EC/WMI. Khi ứng dụng thoát, dịch vụ được bật lại nguyên trạng.

Rust
// src/scm.rs
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::thread::sleep;
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::System::Services::*;

pub struct ServiceGuard {
    service_name: Vec<u16>,
    was_running: bool,
    manage: bool,
}

impl ServiceGuard {
    pub fn acquire(name: &str, manage: bool) -> Self {
        let wide_name: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
        let mut guard = Self {
            service_name: wide_name,
            was_running: false,
            manage,
        };
        if manage {
            guard.stop_if_running();
        }
        guard
    }

    fn stop_if_running(&mut self) {
        unsafe {
            let Ok(scm) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) else { return };
            let Ok(svc) = OpenServiceW(
                scm,
                PCWSTR(self.service_name.as_ptr()),
                SERVICE_QUERY_STATUS | SERVICE_STOP | SERVICE_START,
            ) else {
                let _ = CloseServiceHandle(scm);
                return;
            };

            let mut status = SERVICE_STATUS::default();
            if QueryServiceStatus(svc, &mut status).is_ok() {
                if status.dwCurrentState == SERVICE_RUNNING {
                    self.was_running = true;
                    let _ = ControlService(svc, SERVICE_CONTROL_STOP, &mut status);
                    // Chờ tối đa 3 giây để service dừng hẳn
                    for _ in 0..30 {
                        sleep(Duration::from_millis(100));
                        let _ = QueryServiceStatus(svc, &mut status);
                        if status.dwCurrentState == SERVICE_STOPPED {
                            break;
                        }
                    }
                }
            }
            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);
        }
    }

    pub fn restore(&self) {
        if !self.manage || !self.was_running {
            return;
        }
        unsafe {
            let Ok(scm) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) else { return };
            if let Ok(svc) = OpenServiceW(scm, PCWSTR(self.service_name.as_ptr()), SERVICE_START) {
                let _ = StartServiceW(svc, None);
                let _ = CloseServiceHandle(svc);
            }
            let _ = CloseServiceHandle(scm);
        }
    }
}

impl Drop for ServiceGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

Giai đoạn 4: Hệ thống Điều khiển Quạt Kép - WMI ACPI & Direct EC (src/fan_backend.rs)

Acer Predator sử dụng WMI namespace root\WMI với class AcerGaming_SystemInformation (chứa các method SetGamingFanBehavior, SetGamingFanSpeed, GetGamingSysInfo). Ngoài ra, module hỗ trợ ghi trực tiếp vào cổng ACPI Embedded Controller (0x62 Data / 0x66 Command) thông qua dynamic FFI tới inpoutx64.dll (tránh phải viết kernel driver mới).

Rust
// src/fan_backend.rs
use windows::core::{PCWSTR, s};
use windows::Win32::System::LibraryLoader::{LoadLibraryW, GetProcAddress};

pub trait FanController {
    fn set_Manual_mode(&mut self) -> bool;
    fn set_auto_mode(&mut self) -> bool;
    fn set_speeds(&mut self, cpu_pct: u8, gpu_pct: u8) -> bool;
    fn read_temps(&mut self) -> Option<(u32, u32)>;
}

/// 1. Backend WMI ACPI (root\WMI -> AcerGaming_SystemInformation)
pub struct AcerWmiBackend {
    wmi_con: wmi::WMIConnection,
}

impl AcerWmiBackend {
    pub fn new() -> Option<Self> {
        let com_lib = wmi::COMLibrary::without_security().ok()?;
        let wmi_con = wmi::WMIConnection::with_namespace_path("root\\WMI", com_lib).ok()?;
        Some(Self { wmi_con })
    }

    /// Đóng gói payload 64-bit chuẩn Acer Gaming WMI:
    /// Byte 0: Fan Index (1 = CPU, 2 = GPU1)
    /// Byte 1: Tốc độ quạt (0 - 100%)
    fn encode_fan_speed_arg(fan_index: u8, speed_pct: u8) -> u64 {
        ((speed_pct.min(100) as u64) << 8) | (fan_index as u64)
    }
}

/// 2. Backend Direct EC (Embedded Controller I/O Port 0x62 / 0x66)
type Out32Fn = unsafe extern "system" fn(port: u16, data: u16);
type Inp32Fn = unsafe extern "system" fn(port: u16) -> u16;

pub struct DirectEcBackend {
    out32: Out32Fn,
    inp32: Inp32Fn,
    cpu_speed_reg: u8,
    gpu_speed_reg: u8,
}

impl DirectEcBackend {
    pub fn new(cpu_speed_reg: u8, gpu_speed_reg: u8) -> Option<Self> {
        unsafe {
            let dll_name: Vec<u16> = "inpoutx64.dll\0".encode_utf16().collect();
            let h_mod = LoadLibraryW(PCWSTR(dll_name.as_ptr())).ok()?;
            let out_addr = GetProcAddress(h_mod, s!("Out32"))?;
            let inp_addr = GetProcAddress(h_mod, s!("Inp32"))?;
            Some(Self {
                out32: std::mem::transmute(out_addr),
                inp32: std::mem::transmute(inp_addr),
                cpu_speed_reg,
                gpu_speed_reg,
            })
        }
    }

    unsafe fn wait_ec_ibf_clear(&self) -> bool {
        for _ in 0..1000 {
            // Bit 1 của thanh ghi trạng thái 0x66 là IBF (Input Buffer Full)
            if ((self.inp32)(0x66) & 0x02) == 0 {
                return true;
            }
            std::hint::spin_loop();
        }
        false
    }

    pub fn write_ec_byte(&self, reg: u8, val: u8) -> bool {
        unsafe {
            if !self.wait_ec_ibf_clear() { return false; }
            (self.out32)(0x66, 0x81); // 0x81 = Write EC Command
            if !self.wait_ec_ibf_clear() { return false; }
            (self.out32)(0x62, reg as u16);
            if !self.wait_ec_ibf_clear() { return false; }
            (self.out32)(0x62, val as u16);
            true
        }
    }
}


(Hướng dẫn chi tiết cho Antigravity khi hoàn thiện AcerWmiBackend: Sử dụng IWbemServices::ExecMethod từ windows::Win32::System::Wmi để gọi trực tiếp method SetGamingFanBehavior với tham số uint64 đặt chế độ Custom, và SetGamingFanSpeed để truyền tốc độ quạt, giúp không phát sinh cấp phát bộ nhớ động trong vòng lặp).

Giai đoạn 5: Polling Nhiệt độ Siêu nhẹ (src/sensor.rs)

Để giữ mức sử dụng CPU < 0.1%, tuyệt đối không spawn tiến trình con (nvidia-smi.exe) hay khởi tạo lại COM/WMI mỗi chu kỳ:

Nhiệt độ CPU & Hệ thống: Gọi trực tiếp method GetGamingSysInfo của class AcerGaming_SystemInformation (đã giữ sẵn con trỏ IWbemServices), hoặc đọc thanh ghi nhiệt độ trên EC (0xB0 cho CPU, 0xB1 cho GPU trên dòng Predator Helios/Triton).

Nhiệt độ GPU rời NVIDIA (Fallback chuẩn xác): Nạp động nvml.dll một lần duy nhất lúc khởi động (nvmlInit_v2, nvmlDeviceGetHandleByIndex_v2) và chỉ gọi hàm C nhẹ nvmlDeviceGetTemperature(device, 0, &mut temp) trong mỗi chu kỳ polling (tốn < 5 micro-giây).

Giai đoạn 6: System Tray, Vòng lặp Điều khiển & Cơ chế An toàn (src/tray.rs & src/main.rs)

Bước 6.1: Thiết kế System Tray (src/tray.rs)

Tạo cửa sổ ẩn (HWND_MESSAGE hoặc window class ẩn) để nhận sự kiện WM_APP + 1 từ khay hệ thống.

Menu chuột phải (CreatePopupMenu) gồm:

Status: CPU 54°C (40%) | GPU 48°C (30%) (cập nhật động khi người dùng click chuột phải mở menu, giúp tiết kiệm 100% GDI/UI CPU khi không mở menu).

Mode: Custom Curve (Active)

Mode: Auto (BIOS Default)

Reload config.toml

Exit (Restore PredatorSense)

Bước 6.2: Luồng Worker Điều khiển Quạt & Fail-Safe (src/main.rs)

Sử dụng 2 thread duy nhất:

Main Thread: Chạy GetMessageW (ngủ 100% thời gian khi không có tương tác chuột vào Tray Icon).

Controller Thread: Ngủ theo poll_interval_ms, thức dậy đọc nhiệt độ, tính toán evaluate_curve(), và chỉ gọi lệnh ghi WMI/EC nếu tốc độ quạt mục tiêu khác với tốc độ đã áp dụng ở chu kỳ trước (giảm 90% số lần gọi I/O bus).

Cơ chế An toàn phần cứng (Hardware Fail-Safe): Khi nhận tín hiệu thoát (WM_QUIT, WM_CLOSE, hoặc SetConsoleCtrlHandler), ứng dụng bắt buộc:

Chuyển quạt về chế độ Auto (BIOS Control) trước khi tắt.

Kích hoạt ServiceGuard::restore() để bật lại PredatorSenseService.

4. Quy trình Kiểm thử & Nghiệm thu (Verification Checklist cho Antigravity)

Kỹ sư Antigravity thực thi tuần tự các lệnh kiểm chứng sau trên máy đích:

Biên dịch Release:

PowerShell
cargo build --release


Nghiệm thu: Kiểm tra dung lượng file target\release\predator-fan-ctl.exe < 1.5 MB.

Kiểm tra UAC Manifest & Quản lý Service:

Bật PredatorSenseService trong services.msc.

Chạy predator-fan-ctl.exe. Xác nhận hộp thoại UAC xuất hiện.

Chạy lệnh sc query PredatorSenseService, xác nhận trạng thái chuyển sang STOPPED.

Kiểm tra Hiệu năng (Performance Footprint):

Mở Task Manager / Process Explorer, quan sát tiến trình predator-fan-ctl.exe trong 5 phút với poll_interval_ms = 500.

Nghiệm thu: Private Working Set < 3 MB (mục tiêu < 15 MB), CPU Usage ở mức 0.00% - 0.05% (mục tiêu < 0.1%).

Kiểm tra Phản ứng Tức thì (Zero-Hysteresis) & Fail-Safe:

Chạy bài test tải CPU (như CPU-Z Bench) và chỉnh sửa config.toml hạ điểm kích hoạt 100% xuống 55°C, bấm Reload config.toml trên System Tray. Xác nhận quạt tăng tốc ngay lập tức ở chu kỳ kế tiếp mà không có độ trễ giữ nhiệt.

Bấm Exit trên System Tray, chạy sc query PredatorSenseService và xác nhận dịch vụ đã trở lại trạng thái RUNNING.

Bạn đang sử dụng mẫu máy Acer Predator cụ thể nào (ví dụ: Helios 300 PH315-54, Helios Neo 16, hay Triton 500)? Nếu bạn cung cấp mã model hoặc dump WMI từ máy, tôi có thể xuất ngay mã nguồn hoàn chỉnh của từng file .rs với đúng địa chỉ thanh ghi EC và chữ ký WMI method của dòng máy đó để bạn biên dịch ngay.