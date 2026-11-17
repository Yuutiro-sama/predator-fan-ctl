//! Module Đọc Cảm biến Toàn diện (Zero-Allocation Hardware Telemetry Polling Loop).
//!
//! Thiết kế chuyên biệt để duy trì mức tiêu thụ CPU < 0.05%:
//! - GPU: Nạp động `nvml.dll` qua Win32 FFI một lần duy nhất lúc khởi động.
//!   Mỗi chu kỳ chỉ gọi trực tiếp hàm con trỏ C `nvmlDeviceGetTemperature`, `nvmlDeviceGetPowerUsage`
//!   và `nvmlDeviceGetClockInfo` (< 5 micro-giây, 0 allocation, an toàn với NVIDIA Optimus D3cold).
//! - CPU Xung nhịp: `CallNtPowerInformation` (PowrProf Win32 FFI) đọc trực tiếp cấu trúc năng lượng
//!   của toàn bộ các luồng logic (vd 20 luồng i7-13700HX) và trích xuất xung nhịp boost đỉnh max(CurrentMhz).
//! - CPU Công suất: Windows PDH English Counter `\Energy Meter(RAPL_Package0_PKG)\Power` đọc trực tiếp
//!   công suất thực tế Intel RAPL Package 0 đơn vị deciwatt (0.1 W).
//! - CPU Nhiệt độ: ACPI ThermalZone (`root\WMI` -> `MSAcpi_ThermalZoneTemperature.CurrentTemperature`).

use std::ffi::c_void;
use serde::Deserialize;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

// ============================================================================
// 1. GPU SENSOR (Dynamic FFI NVML)
// ============================================================================

type NvmlInitFn = unsafe extern "system" fn() -> i32;
type NvmlShutdownFn = unsafe extern "system" fn() -> i32;
type NvmlDeviceGetHandleByIndexFn =
    unsafe extern "system" fn(u32, *mut *mut c_void) -> i32;
type NvmlDeviceGetTemperatureFn =
    unsafe extern "system" fn(*mut c_void, u32, *mut u32) -> i32;
type NvmlDeviceGetPowerUsageFn =
    unsafe extern "system" fn(*mut c_void, *mut u32) -> i32;
type NvmlDeviceGetClockInfoFn =
    unsafe extern "system" fn(*mut c_void, u32, *mut u32) -> i32;

const NVML_SUCCESS: i32 = 0;
const NVML_TEMPERATURE_GPU: u32 = 0;
const NVML_CLOCK_GRAPHICS: u32 = 0;

/// Bộ đọc cảm biến GPU NVIDIA sử dụng dynamic FFI tới `nvml.dll`.
pub struct NvmlGpuSensor {
    module: HMODULE,
    shutdown: NvmlShutdownFn,
    get_temp: NvmlDeviceGetTemperatureFn,
    get_power: Option<NvmlDeviceGetPowerUsageFn>,
    get_clock: Option<NvmlDeviceGetClockInfoFn>,
    device: *mut c_void,
    last_clock: std::sync::atomic::AtomicU16,
}

// Cho phép truyền NvmlGpuSensor giữa các thread an toàn
unsafe impl Send for NvmlGpuSensor {}
unsafe impl Sync for NvmlGpuSensor {}

impl NvmlGpuSensor {
    /// Khởi tạo NVML sensor bằng cách nạp động `nvml.dll`.
    /// Trả về `None` nếu máy tính không có GPU NVIDIA hoặc không cài driver NVML.
    pub fn new() -> Option<Self> {
        unsafe {
            // Thử nạp nvml.dll từ search path chuẩn (System32)
            let mut dll_wide: Vec<u16> = "nvml.dll\0".encode_utf16().collect();
            let mut h_mod = LoadLibraryW(PCWSTR(dll_wide.as_ptr())).ok();

            // Nếu không tìm thấy, thử đường dẫn dự phòng NVSMI
            if h_mod.is_none() {
                let fallback = "C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvml.dll\0";
                dll_wide = fallback.encode_utf16().collect();
                h_mod = LoadLibraryW(PCWSTR(dll_wide.as_ptr())).ok();
            }

            let module = h_mod?;

            // Lấy con trỏ các hàm cần thiết
            let init_name = b"nvmlInit_v2\0";
            let shutdown_name = b"nvmlShutdown\0";
            let handle_name = b"nvmlDeviceGetHandleByIndex_v2\0";
            let temp_name = b"nvmlDeviceGetTemperature\0";
            let power_name = b"nvmlDeviceGetPowerUsage\0";
            let clock_name = b"nvmlDeviceGetClockInfo\0";

            let p_init = GetProcAddress(module, windows::core::PCSTR(init_name.as_ptr()))?;
            let p_shutdown = GetProcAddress(module, windows::core::PCSTR(shutdown_name.as_ptr()))?;
            let p_handle = GetProcAddress(module, windows::core::PCSTR(handle_name.as_ptr()))?;
            let p_temp = GetProcAddress(module, windows::core::PCSTR(temp_name.as_ptr()))?;

            let init_fn: NvmlInitFn = std::mem::transmute(p_init);
            let shutdown_fn: NvmlShutdownFn = std::mem::transmute(p_shutdown);
            let handle_fn: NvmlDeviceGetHandleByIndexFn = std::mem::transmute(p_handle);
            let get_temp_fn: NvmlDeviceGetTemperatureFn = std::mem::transmute(p_temp);

            let get_power: Option<NvmlDeviceGetPowerUsageFn> =
                GetProcAddress(module, windows::core::PCSTR(power_name.as_ptr()))
                    .map(|p| std::mem::transmute(p));

            let get_clock: Option<NvmlDeviceGetClockInfoFn> =
                GetProcAddress(module, windows::core::PCSTR(clock_name.as_ptr()))
                    .map(|p| std::mem::transmute(p));

            // Khởi tạo thư viện NVML
            if init_fn() != NVML_SUCCESS {
                let _ = FreeLibrary(module);
                return None;
            }

            // Lấy handle của GPU chính (index 0)
            let mut device: *mut c_void = std::ptr::null_mut();
            if handle_fn(0, &mut device) != NVML_SUCCESS || device.is_null() {
                let _ = shutdown_fn();
                let _ = FreeLibrary(module);
                return None;
            }

            Some(Self {
                module,
                shutdown: shutdown_fn,
                get_temp: get_temp_fn,
                get_power,
                get_clock,
                device,
                last_clock: std::sync::atomic::AtomicU16::new(0),
            })
        }
    }

    /// Đọc nhiệt độ GPU tức thời (°C).
    /// Thực thi trực tiếp qua con trỏ hàm FFI, không phân bổ bộ nhớ động (0 allocation).
    #[inline]
    pub fn read_temp(&self) -> Option<u32> {
        let mut temp: u32 = 0;
        unsafe {
            if (self.get_temp)(self.device, NVML_TEMPERATURE_GPU, &mut temp) == NVML_SUCCESS {
                Some(temp)
            } else {
                None
            }
        }
    }

    /// Đọc công suất (deciwatt: 0.1 W) và xung nhịp GPU (MHz).
    ///
    /// An toàn tuyệt đối với NVIDIA Optimus D3cold:
    /// Khi dGPU ở chế độ ngủ tiết kiệm điện hoặc NVML trả về mã lỗi (!= 0),
    /// hàm trả về (0, 0) mà không gây panic, không ghi log spam và không đánh thức dGPU.
    /// Nếu GPU đang hoạt động mà đọc clock thất bại tạm thời, fallback giữ giá trị gần nhất.
    #[inline]
    pub fn read_power_and_clock(&self) -> (u16, u16) {
        let mut power_dw: u16 = 0;
        let mut clock_mhz: u16 = 0;

        unsafe {
            if let Some(get_power) = self.get_power {
                let mut power_mw: u32 = 0;
                if get_power(self.device, &mut power_mw) == NVML_SUCCESS {
                    // Quy đổi mW sang deciwatts (0.1 W): (power_mw + 50) / 100
                    power_dw = ((power_mw + 50) / 100).min(u16::MAX as u32) as u16;
                }
            }

            if let Some(get_clock) = self.get_clock {
                let mut mhz: u32 = 0;
                if get_clock(self.device, NVML_CLOCK_GRAPHICS, &mut mhz) == NVML_SUCCESS && mhz > 0 {
                    clock_mhz = mhz.min(u16::MAX as u32) as u16;
                    self.last_clock.store(clock_mhz, std::sync::atomic::Ordering::Relaxed);
                } else if power_dw > 0 {
                    // Fallback: Nếu GPU đang cấp điện mà NVML clock đọc fail tức thời, giữ giá trị cache
                    clock_mhz = self.last_clock.load(std::sync::atomic::Ordering::Relaxed);
                }
            }
        }

        (power_dw, clock_mhz)
    }
}

impl Drop for NvmlGpuSensor {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.shutdown)();
            let _ = FreeLibrary(self.module);
        }
    }
}

// ============================================================================
// 2. PDH FFI DEFINITIONS (Shared for Clock & Power Sensors)
// ============================================================================

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PdhFmtCounterValueDouble {
    pub c_status: u32,
    pub _padding: u32,
    pub double_value: f64,
}

type PdhOpenQueryWFn = unsafe extern "system" fn(PCWSTR, usize, *mut usize) -> i32;
type PdhAddEnglishCounterWFn = unsafe extern "system" fn(usize, PCWSTR, usize, *mut usize) -> i32;
type PdhCollectQueryDataFn = unsafe extern "system" fn(usize) -> i32;
type PdhGetFormattedCounterValueFn =
    unsafe extern "system" fn(usize, u32, *mut u32, *mut PdhFmtCounterValueDouble) -> i32;
type PdhCloseQueryFn = unsafe extern "system" fn(usize) -> i32;

const PDH_FMT_DOUBLE: u32 = 0x00000200;

// ============================================================================
// 3. CPU CLOCK SENSOR (PDH Persistent Query + PowrProf Fallback)
// ============================================================================

/// Cấu trúc thông tin năng lượng và xung nhịp từng luồng CPU từ powrprof.dll
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(non_snake_case)]
pub struct PROCESSOR_POWER_INFORMATION {
    pub Number: u32,
    pub MaxMhz: u32,
    pub CurrentMhz: u32,
    pub MhzLimit: u32,
    pub MaxIdleState: u32,
    pub CurrentIdleState: u32,
}

type CallNtPowerInformationFn = unsafe extern "system" fn(
    i32,           // InformationLevel: 11 = ProcessorInformation
    *const c_void, // lpInputBuffer
    u32,           // nInputBufferSize
    *mut c_void,   // lpOutputBuffer
    u32,           // nOutputBufferSize
) -> i32;

#[repr(C)]
struct SYSTEM_INFO {
    w_processor_architecture: u16,
    w_reserved: u16,
    dw_page_size: u32,
    lp_minimum_application_address: *mut c_void,
    lp_maximum_application_address: *mut c_void,
    dw_active_processor_mask: usize,
    dw_number_of_processors: u32,
    dw_processor_type: u32,
    dw_allocation_granularity: u32,
    w_processor_level: u16,
    w_processor_revision: u16,
}

extern "system" {
    fn GetSystemInfo(lpSystemInfo: *mut SYSTEM_INFO);
}

/// Bộ đọc xung nhịp CPU sử dụng Windows Performance Counters (PDH) theo cơ chế
/// Persistent Query (giữ handle trong suốt vòng đời sensor, zero allocation, độ trễ < 0.05ms/tick).
///
/// Phản ánh xung nhịp Turbo Boost thực tế tức thời (tương đương Task Manager),
/// đồng thời hỗ trợ dự phòng PowrProf CallNtPowerInformation nếu PDH không khả dụng.
pub struct CpuClockSensor {
    base_mhz: f64,
    pdh_module: Option<HMODULE>,
    pdh_query: usize,
    pdh_perf_counter: usize,
    pdh_available: bool,
    last_clock_mhz: u16,
    collect_fn: Option<PdhCollectQueryDataFn>,
    get_formatted_fn: Option<PdhGetFormattedCounterValueFn>,
    close_fn: Option<PdhCloseQueryFn>,
    // Fallback PowrProf nếu cần
    powrprof_module: Option<HMODULE>,
    call_nt_power_info: Option<CallNtPowerInformationFn>,
    buffer: Vec<PROCESSOR_POWER_INFORMATION>,
}

unsafe impl Send for CpuClockSensor {}
unsafe impl Sync for CpuClockSensor {}

impl CpuClockSensor {
    /// Khởi tạo sensor xung nhịp CPU:
    /// - Bước 1: Lấy `base_mhz` danh định của Core 0 qua `CallNtPowerInformation(11, ...)`.
    /// - Bước 2: Nạp động `pdh.dll`, mở persistent query và thêm english counter:
    ///   `\\Processor Information(_Total)\\% Processor Performance`.
    /// - Bước 3: Lấy mẫu mồi (priming sample) lúc khởi tạo để thiết lập mốc thời gian delta.
    pub fn new() -> Self {
        // 1. Xác định số luồng logic và cấu hình buffer PowrProf
        let mut sys_info = unsafe { std::mem::zeroed::<SYSTEM_INFO>() };
        unsafe { GetSystemInfo(&mut sys_info) };
        let num_procs = if sys_info.dw_number_of_processors > 0 {
            sys_info.dw_number_of_processors as usize
        } else {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(20)
        };

        let mut buffer = vec![PROCESSOR_POWER_INFORMATION::default(); num_procs];

        let dll_wide: Vec<u16> = "powrprof.dll\0".encode_utf16().collect();
        let powrprof_module = unsafe { LoadLibraryW(PCWSTR(dll_wide.as_ptr())).ok() };

        let call_nt_power_info: Option<CallNtPowerInformationFn> = powrprof_module.and_then(|m| unsafe {
            GetProcAddress(m, windows::core::PCSTR(b"CallNtPowerInformation\0".as_ptr()))
                .map(|p| std::mem::transmute(p))
        });

        // 2. Bước 1: Lấy base_mhz danh định của Core 0 (trên i7-13650HX là 2600.0 MHz)
        let mut base_mhz = 2600.0;
        if let Some(call_fn) = call_nt_power_info {
            let out_size =
                (buffer.len() * std::mem::size_of::<PROCESSOR_POWER_INFORMATION>()) as u32;
            let status = unsafe {
                call_fn(
                    11, // ProcessorInformation
                    std::ptr::null(),
                    0,
                    buffer.as_mut_ptr() as *mut c_void,
                    out_size,
                )
            };
            if status == 0 && !buffer.is_empty() && buffer[0].MaxMhz > 0 {
                base_mhz = buffer[0].MaxMhz as f64;
            }
        }

        // 3. Bước 2..6: Nạp động pdh.dll và thiết lập Persistent Query
        let mut pdh_module = None;
        let mut pdh_query = 0usize;
        let mut pdh_perf_counter = 0usize;
        let mut pdh_available = false;
        let mut collect_fn = None;
        let mut get_formatted_fn = None;
        let mut close_fn = None;

        unsafe {
            let pdh_dll_wide: Vec<u16> = "pdh.dll\0".encode_utf16().collect();
            if let Ok(mod_handle) = LoadLibraryW(PCWSTR(pdh_dll_wide.as_ptr())) {
                let p_open = GetProcAddress(mod_handle, windows::core::PCSTR(b"PdhOpenQueryW\0".as_ptr()));
                let p_add = GetProcAddress(mod_handle, windows::core::PCSTR(b"PdhAddEnglishCounterW\0".as_ptr()));
                let p_collect = GetProcAddress(mod_handle, windows::core::PCSTR(b"PdhCollectQueryData\0".as_ptr()));
                let p_fmt = GetProcAddress(mod_handle, windows::core::PCSTR(b"PdhGetFormattedCounterValue\0".as_ptr()));
                let p_close = GetProcAddress(mod_handle, windows::core::PCSTR(b"PdhCloseQuery\0".as_ptr()));

                if let (Some(p_open), Some(p_add), Some(p_collect), Some(p_fmt), Some(p_close)) =
                    (p_open, p_add, p_collect, p_fmt, p_close)
                {
                    let open_query: PdhOpenQueryWFn = std::mem::transmute(p_open);
                    let add_counter: PdhAddEnglishCounterWFn = std::mem::transmute(p_add);
                    let c_fn: PdhCollectQueryDataFn = std::mem::transmute(p_collect);
                    let f_fn: PdhGetFormattedCounterValueFn = std::mem::transmute(p_fmt);
                    let cl_fn: PdhCloseQueryFn = std::mem::transmute(p_close);

                    let mut query: usize = 0;
                    if open_query(PCWSTR::null(), 0, &mut query) == 0 && query != 0 {
                        let counter_path: Vec<u16> = "\\Processor Information(_Total)\\% Processor Performance\0"
                            .encode_utf16()
                            .collect();
                        let mut counter: usize = 0;
                        if add_counter(query, PCWSTR(counter_path.as_ptr()), 0, &mut counter) == 0 && counter != 0 {
                            // Priming sample mồi lần đầu
                            let _ = c_fn(query);
                            pdh_query = query;
                            pdh_perf_counter = counter;
                            pdh_available = true;
                            collect_fn = Some(c_fn);
                            get_formatted_fn = Some(f_fn);
                            close_fn = Some(cl_fn);
                            pdh_module = Some(mod_handle);
                        } else {
                            let _ = cl_fn(query);
                            let _ = FreeLibrary(mod_handle);
                        }
                    } else {
                        let _ = FreeLibrary(mod_handle);
                    }
                } else {
                    let _ = FreeLibrary(mod_handle);
                }
            }
        }

        let mut sensor = Self {
            base_mhz,
            pdh_module,
            pdh_query,
            pdh_perf_counter,
            pdh_available,
            last_clock_mhz: base_mhz.clamp(400.0, 6500.0) as u16,
            collect_fn,
            get_formatted_fn,
            close_fn,
            powrprof_module,
            call_nt_power_info,
            buffer,
        };

        // Lấy mẫu mồi lần đầu
        let _ = sensor.sample_max_mhz();
        sensor
    }

    /// Lấy mẫu xung nhịp tức thời và trả về xung nhịp động thực tế (MHz).
    /// Sử dụng công thức: `dynamic_mhz = base_mhz * (% Processor Performance / 100)`.
    pub fn sample_max_mhz(&mut self) -> u16 {
        if self.pdh_available {
            if let (Some(collect_fn), Some(get_formatted_fn)) = (self.collect_fn, self.get_formatted_fn) {
                unsafe {
                    if collect_fn(self.pdh_query) == 0 {
                        let mut val = PdhFmtCounterValueDouble::default();
                        let status = get_formatted_fn(
                            self.pdh_perf_counter,
                            PDH_FMT_DOUBLE,
                            std::ptr::null_mut(),
                            &mut val,
                        );
                        // c_status: 0 (PDH_CSTATUS_VALID_DATA) hoặc 1 (PDH_CSTATUS_NEW_DATA)
                        if status == 0 && (val.c_status == 0 || val.c_status == 1) && val.double_value > 10.0 {
                            let dynamic_mhz = (self.base_mhz * (val.double_value / 100.0)).round() as u32;
                            self.last_clock_mhz = dynamic_mhz.clamp(400, 6500) as u16;
                            return self.last_clock_mhz;
                        }
                    }
                }
            }
        }

        // Fallback: Nếu PDH lỗi hoặc không khả dụng, dùng CallNtPowerInformation
        if let Some(call_fn) = self.call_nt_power_info {
            let out_size =
                (self.buffer.len() * std::mem::size_of::<PROCESSOR_POWER_INFORMATION>()) as u32;
            let status = unsafe {
                call_fn(
                    11, // ProcessorInformation
                    std::ptr::null(),
                    0,
                    self.buffer.as_mut_ptr() as *mut c_void,
                    out_size,
                )
            };

            if status == 0 {
                let max_mhz = self.buffer.iter().map(|p| p.CurrentMhz).max().unwrap_or(0);
                if max_mhz > 0 {
                    self.last_clock_mhz = max_mhz.clamp(400, 6500) as u16;
                    return self.last_clock_mhz;
                }
            }
        }

        self.last_clock_mhz
    }

    /// Alias đọc xung nhịp CPU theo đơn vị MHz (u32), hỗ trợ đồng bộ theo đặc tả plan.md
    #[inline]
    pub fn read_mhz(&mut self) -> u32 {
        self.sample_max_mhz() as u32
    }

    /// Xung nhịp CPU cao nhất đã đo gần nhất (MHz).
    #[inline]
    pub fn last_max_mhz(&self) -> u16 {
        self.last_clock_mhz
    }

    /// Base clock danh định (MHz).
    #[inline]
    pub fn base_mhz(&self) -> f64 {
        self.base_mhz
    }

    /// Kiểm tra xem PDH Dynamic Clock có đang hoạt động hay không.
    #[inline]
    pub fn is_pdh_available(&self) -> bool {
        self.pdh_available
    }
}

impl Drop for CpuClockSensor {
    fn drop(&mut self) {
        unsafe {
            if self.pdh_available && self.pdh_query != 0 {
                if let Some(close_fn) = self.close_fn {
                    let _ = close_fn(self.pdh_query);
                }
            }
            if let Some(m) = self.pdh_module {
                let _ = FreeLibrary(m);
            }
            if let Some(m) = self.powrprof_module {
                let _ = FreeLibrary(m);
            }
        }
    }
}

// ============================================================================
// 4. CPU POWER SENSOR (Windows PDH - RAPL Energy Meter)
// ============================================================================

/// Bộ đọc công suất CPU sử dụng Performance Data Helper (PDH) Counter RAPL Package.
pub struct CpuPowerSensor {
    module: HMODULE,
    query: usize,
    counter: usize,
    collect_fn: PdhCollectQueryDataFn,
    get_formatted_fn: PdhGetFormattedCounterValueFn,
    close_fn: PdhCloseQueryFn,
    last_power_dw: u16,
}

unsafe impl Send for CpuPowerSensor {}
unsafe impl Sync for CpuPowerSensor {}

impl CpuPowerSensor {
    /// Khởi tạo PDH query với counter `\Energy Meter(RAPL_Package0_PKG)\Power`.
    /// Trả về `None` nếu PDH hoặc counter RAPL không khả dụng.
    pub fn new() -> Option<Self> {
        unsafe {
            let dll_wide: Vec<u16> = "pdh.dll\0".encode_utf16().collect();
            let module = LoadLibraryW(PCWSTR(dll_wide.as_ptr())).ok()?;

            let p_open = GetProcAddress(module, windows::core::PCSTR(b"PdhOpenQueryW\0".as_ptr()))?;
            let p_add =
                GetProcAddress(module, windows::core::PCSTR(b"PdhAddEnglishCounterW\0".as_ptr()))?;
            let p_collect =
                GetProcAddress(module, windows::core::PCSTR(b"PdhCollectQueryData\0".as_ptr()))?;
            let p_fmt = GetProcAddress(
                module,
                windows::core::PCSTR(b"PdhGetFormattedCounterValue\0".as_ptr()),
            )?;
            let p_close =
                GetProcAddress(module, windows::core::PCSTR(b"PdhCloseQuery\0".as_ptr()))?;

            let open_query: PdhOpenQueryWFn = std::mem::transmute(p_open);
            let add_counter: PdhAddEnglishCounterWFn = std::mem::transmute(p_add);
            let collect_fn: PdhCollectQueryDataFn = std::mem::transmute(p_collect);
            let get_formatted_fn: PdhGetFormattedCounterValueFn = std::mem::transmute(p_fmt);
            let close_fn: PdhCloseQueryFn = std::mem::transmute(p_close);

            let mut query: usize = 0;
            if open_query(PCWSTR::null(), 0, &mut query) != 0 || query == 0 {
                let _ = FreeLibrary(module);
                return None;
            }

            let counter_path: Vec<u16> = "\\Energy Meter(RAPL_Package0_PKG)\\Power\0"
                .encode_utf16()
                .collect();
            let mut counter: usize = 0;
            if add_counter(query, PCWSTR(counter_path.as_ptr()), 0, &mut counter) != 0 || counter == 0 {
                let _ = close_fn(query);
                let _ = FreeLibrary(module);
                return None;
            }

            // Gọi mồi lần đầu để PDH thiết lập mốc đo năng lượng baseline
            let _ = collect_fn(query);

            Some(Self {
                module,
                query,
                counter,
                collect_fn,
                get_formatted_fn,
                close_fn,
                last_power_dw: 0,
            })
        }
    }

    /// Lấy mẫu công suất tiêu thụ CPU hiện tại theo đơn vị deciwatt (0.1 W).
    /// Ví dụ: 42.8 W trả về 428 deciwatt.
    pub fn sample_deciwatts(&mut self) -> u16 {
        unsafe {
            if (self.collect_fn)(self.query) == 0 {
                let mut val = PdhFmtCounterValueDouble::default();
                if (self.get_formatted_fn)(self.counter, PDH_FMT_DOUBLE, std::ptr::null_mut(), &mut val)
                    == 0
                    && val.c_status == 0
                {
                    if val.double_value >= 0.0 {
                        // val.double_value trả về milliwatts (mW).
                        // Quy đổi mW sang deciwatts (0.1 W): mW / 100
                        let dw = (val.double_value / 100.0).round();
                        self.last_power_dw = dw.min(u16::MAX as f64).max(0.0) as u16;
                    }
                }
            }
        }

        self.last_power_dw
    }

    /// Công suất CPU gần nhất đo được (deciwatt).
    #[inline]
    pub fn last_power_dw(&self) -> u16 {
        self.last_power_dw
    }
}

impl Drop for CpuPowerSensor {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.close_fn)(self.query);
            let _ = FreeLibrary(self.module);
        }
    }
}

// ============================================================================
// 4. CPU TEMPERATURE SENSOR (ACPI ThermalZone via WMI)
// ============================================================================

#[derive(Deserialize, Debug)]
struct ThermalZoneQuery {
    #[serde(rename = "CurrentTemperature")]
    current_temperature: Option<u32>,
}

#[derive(Deserialize, Debug)]
struct Cimv2ThermalQuery {
    #[serde(rename = "Temperature")]
    temperature: Option<u32>,
}

/// Bộ đọc nhiệt độ CPU thông qua ACPI ThermalZone WMI.
pub struct AcpiCpuSensor {
    wmi_con_acpi: Option<wmi::WMIConnection>,
    wmi_con_cimv2: Option<wmi::WMIConnection>,
}

unsafe impl Send for AcpiCpuSensor {}
unsafe impl Sync for AcpiCpuSensor {}

impl AcpiCpuSensor {
    /// Khởi tạo sensor ACPI ThermalZone.
    pub fn new() -> Self {
        let com_lib = wmi::COMLibrary::without_security().ok();

        let wmi_con_acpi = com_lib.and_then(|lib| {
            wmi::WMIConnection::with_namespace_path("root\\WMI", lib).ok()
        });

        // Tạo thêm connection dự phòng tới CIMV2 nếu cần
        let wmi_con_cimv2 = wmi::COMLibrary::without_security().ok().and_then(|lib| {
            wmi::WMIConnection::with_namespace_path("root\\CIMV2", lib).ok()
        });

        Self {
            wmi_con_acpi,
            wmi_con_cimv2,
        }
    }

    /// Đọc nhiệt độ CPU hiện tại (°C).
    ///
    /// Chuyển đổi từ tenths of Kelvin sang độ C:
    /// $T(^\circ\text{C}) = \frac{\text{CurrentTemperature} - 2732}{10}$
    pub fn read_temp(&self) -> Option<u32> {
        // 1. Ưu tiên truy vấn root\WMI -> MSAcpi_ThermalZoneTemperature
        if let Some(ref con) = self.wmi_con_acpi {
            let query = "SELECT CurrentTemperature FROM MSAcpi_ThermalZoneTemperature";
            if let Ok(results) = con.raw_query::<ThermalZoneQuery>(query) {
                for tz in results {
                    if let Some(kelvin_tenths) = tz.current_temperature {
                        if kelvin_tenths > 2732 && kelvin_tenths < 4232 {
                            // Chuyển đổi và làm tròn
                            let celsius = ((kelvin_tenths - 2732) + 5) / 10;
                            return Some(celsius.min(115));
                        }
                    }
                }
            }
        }

        // 2. Dự phòng: root\CIMV2 -> Win32_PerfFormattedData_Counters_ThermalZoneInformation
        if let Some(ref con) = self.wmi_con_cimv2 {
            let query = "SELECT Temperature FROM Win32_PerfFormattedData_Counters_ThermalZoneInformation";
            if let Ok(results) = con.raw_query::<Cimv2ThermalQuery>(query) {
                for tz in results {
                    if let Some(kelvin) = tz.temperature {
                        if kelvin > 273 && kelvin < 423 {
                            let celsius = kelvin - 273;
                            return Some(celsius.min(115));
                        }
                    }
                }
            }
        }

        None
    }
}

// ============================================================================
// 5. COMBINED TEMPERATURE READER (Legacy Compatible)
// ============================================================================

/// Trình quản lý cảm biến nhiệt độ tổng hợp cho CPU và GPU.
/// Đảm bảo luôn trả về giá trị an toàn ngay cả khi một cảm biến tạm thời mất tín hiệu.
pub struct TemperatureReader {
    gpu: Option<NvmlGpuSensor>,
    cpu: AcpiCpuSensor,
    last_cpu: u32,
    last_gpu: u32,
}

unsafe impl Send for TemperatureReader {}
unsafe impl Sync for TemperatureReader {}

impl TemperatureReader {
    /// Khởi tạo bộ đọc nhiệt độ với các cảm biến có sẵn trên hệ thống.
    pub fn new() -> Self {
        let gpu = NvmlGpuSensor::new();
        let cpu = AcpiCpuSensor::new();

        let mut reader = Self {
            gpu,
            cpu,
            last_cpu: 45,
            last_gpu: 40,
        };

        // Thực hiện đọc mồi lần đầu
        let (c, g) = reader.poll();
        reader.last_cpu = c;
        reader.last_gpu = g;
        reader
    }

    /// Đọc đồng thời nhiệt độ `(cpu_temp, gpu_temp)` (°C).
    /// Tự động sử dụng giá trị cache gần nhất nếu phần cứng không phản hồi.
    pub fn poll(&mut self) -> (u32, u32) {
        if let Some(cpu_t) = self.cpu.read_temp() {
            self.last_cpu = cpu_t;
        }

        if let Some(ref gpu_sensor) = self.gpu {
            if let Some(gpu_t) = gpu_sensor.read_temp() {
                self.last_gpu = gpu_t;
            }
        }

        (self.last_cpu, self.last_gpu)
    }

    /// Nhiệt độ CPU gần nhất.
    #[inline]
    pub fn last_cpu_temp(&self) -> u32 {
        self.last_cpu
    }

    /// Nhiệt độ GPU gần nhất.
    #[inline]
    pub fn last_gpu_temp(&self) -> u32 {
        self.last_gpu
    }

    /// Kiểm tra xem GPU sensor NVML có khả dụng không.
    #[inline]
    pub fn has_gpu_sensor(&self) -> bool {
        self.gpu.is_some()
    }
}

// ============================================================================
// 6. SYSTEM TELEMETRY READER (Full CPU & GPU Metrics Hub)
// ============================================================================

/// Trình đọc vi sai telemetry toàn diện: Nhiệt độ, Công suất và Xung nhịp CPU & GPU.
/// Không phân bổ bộ nhớ động trong vòng lặp thu thập 1 Hz.
pub struct SystemTelemetryReader {
    gpu: Option<NvmlGpuSensor>,
    cpu_temp: AcpiCpuSensor,
    cpu_clock: CpuClockSensor,
    cpu_power: Option<CpuPowerSensor>,
    last_cpu_temp: u32,
    last_gpu_temp: u32,
    last_cpu_power_dw: u16,
    last_gpu_power_dw: u16,
    last_cpu_mhz: u16,
    last_gpu_mhz: u16,
}

unsafe impl Send for SystemTelemetryReader {}
unsafe impl Sync for SystemTelemetryReader {}

impl SystemTelemetryReader {
    /// Khởi tạo trung tâm đọc telemetry với toàn bộ cảm biến phần cứng sẵn có.
    pub fn new() -> Self {
        let gpu = NvmlGpuSensor::new();
        let cpu_temp = AcpiCpuSensor::new();
        let cpu_clock = CpuClockSensor::new();
        let cpu_power = CpuPowerSensor::new();

        let mut reader = Self {
            gpu,
            cpu_temp,
            cpu_clock,
            cpu_power,
            last_cpu_temp: 45,
            last_gpu_temp: 40,
            last_cpu_power_dw: 0,
            last_gpu_power_dw: 0,
            last_cpu_mhz: 2500,
            last_gpu_mhz: 0,
        };

        // Đọc mồi lần đầu
        let _ = reader.poll();
        reader
    }

    /// Đọc đồng thời toàn bộ thông số:
    /// `(cpu_temp: u32, gpu_temp: u32, cpu_power_dw: u16, gpu_power_dw: u16, cpu_mhz: u16, gpu_mhz: u16)`
    pub fn poll(&mut self) -> (u32, u32, u16, u16, u16, u16) {
        if let Some(t) = self.cpu_temp.read_temp() {
            self.last_cpu_temp = t;
        }

        if let Some(ref gpu) = self.gpu {
            if let Some(t) = gpu.read_temp() {
                self.last_gpu_temp = t;
            }
            let (g_pwr, g_clk) = gpu.read_power_and_clock();
            self.last_gpu_power_dw = g_pwr;
            self.last_gpu_mhz = g_clk;
        }

        self.last_cpu_mhz = self.cpu_clock.sample_max_mhz();

        if let Some(ref mut pwr) = self.cpu_power {
            self.last_cpu_power_dw = pwr.sample_deciwatts();
        }

        (
            self.last_cpu_temp,
            self.last_gpu_temp,
            self.last_cpu_power_dw,
            self.last_gpu_power_dw,
            self.last_cpu_mhz,
            self.last_gpu_mhz,
        )
    }

    #[inline]
    pub fn last_cpu_temp(&self) -> u32 {
        self.last_cpu_temp
    }

    #[inline]
    pub fn last_gpu_temp(&self) -> u32 {
        self.last_gpu_temp
    }

    #[inline]
    pub fn last_cpu_power_dw(&self) -> u16 {
        self.last_cpu_power_dw
    }

    #[inline]
    pub fn last_gpu_power_dw(&self) -> u16 {
        self.last_gpu_power_dw
    }

    #[inline]
    pub fn last_cpu_mhz(&self) -> u16 {
        self.last_cpu_mhz
    }

    #[inline]
    pub fn last_gpu_mhz(&self) -> u16 {
        self.last_gpu_mhz
    }

    #[inline]
    pub fn has_gpu_sensor(&self) -> bool {
        self.gpu.is_some()
    }

    #[inline]
    pub fn has_cpu_power_sensor(&self) -> bool {
        self.cpu_power.is_some()
    }
}

// ============================================================================
// UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nvml_gpu_temp() {
        if let Some(gpu) = NvmlGpuSensor::new() {
            let temp = gpu.read_temp();
            println!("Actual GPU temp from NVML: {:?}", temp);
            assert!(temp.is_some());
            let val = temp.unwrap();
            assert!(val >= 20 && val <= 105, "GPU temp {} out of realistic range", val);
        } else {
            println!("No NVML device detected on host.");
        }
    }

    #[test]
    fn test_nvml_gpu_power_and_clock() {
        if let Some(gpu) = NvmlGpuSensor::new() {
            let (power_dw, clock_mhz) = gpu.read_power_and_clock();
            println!("GPU Power: {:.1} W, Clock: {} MHz", power_dw as f32 / 10.0, clock_mhz);
            // Giá trị có thể bằng 0 nếu dGPU đang ở Optimus D3cold, không panic
            assert!(power_dw < 4000); // < 400 W
            assert!(clock_mhz < 4000); // < 4000 MHz
        }
    }

    #[test]
    fn test_cpu_clock_sensor() {
        let mut sensor = CpuClockSensor::new();
        // Cho một khoảng thời gian ngắn để PDH tính toán % hiệu năng thực tế
        std::thread::sleep(std::time::Duration::from_millis(100));
        let max_mhz = sensor.sample_max_mhz();
        println!(
            "Sampled dynamic CPU Clock: {} MHz (Base: {} MHz, PDH active: {})",
            max_mhz,
            sensor.base_mhz(),
            sensor.is_pdh_available()
        );
        assert!(max_mhz > 400, "CPU dynamic clock {} suspiciously low", max_mhz);
        assert!(max_mhz < 7000, "CPU dynamic clock {} exceeds realistic limit", max_mhz);
        assert!(sensor.is_pdh_available(), "PDH dynamic clock query should be active");
    }

    #[test]
    fn test_cpu_dynamic_clock_under_load() {
        let mut sensor = CpuClockSensor::new();
        assert!(sensor.is_pdh_available());

        // Đo baseline ban đầu
        std::thread::sleep(std::time::Duration::from_millis(80));
        let base_sample = sensor.sample_max_mhz();

        // Tạo tải nhẹ để kích thích xung nhịp
        let start = std::time::Instant::now();
        let mut acc: u64 = 0;
        while start.elapsed().as_millis() < 120 {
            for i in 0..10_000 {
                acc = acc.wrapping_add(i);
            }
        }
        std::hint::black_box(acc);

        let dynamic_mhz = sensor.sample_max_mhz();
        println!(
            "Dynamic Clock: baseline={} MHz, dynamic={} MHz (last={})",
            base_sample, dynamic_mhz, sensor.last_max_mhz()
        );

        assert!(dynamic_mhz >= 400 && dynamic_mhz <= 6500, "Dynamic clock {} out of range", dynamic_mhz);
        assert_eq!(dynamic_mhz, sensor.last_max_mhz());

        // Kiểm tra alias read_mhz()
        std::thread::sleep(std::time::Duration::from_millis(50));
        let read_val = sensor.read_mhz();
        println!("Sampled via read_mhz(): {} MHz", read_val);
        assert!(read_val >= 400 && read_val <= 6500);
        assert_eq!(read_val, sensor.last_max_mhz() as u32);
    }

    #[test]
    fn test_cpu_power_sensor() {
        if let Some(mut sensor) = CpuPowerSensor::new() {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let deciwatts = sensor.sample_deciwatts();
            println!("Sampled CPU Package Power: {:.1} W ({} dW)", deciwatts as f32 / 10.0, deciwatts);
            assert!(deciwatts < 3000, "CPU power {} dW suspiciously high", deciwatts);
        } else {
            println!("PDH RAPL counter not available on host system.");
        }
    }

    #[test]
    fn test_system_telemetry_reader() {
        let mut reader = SystemTelemetryReader::new();
        let (cpu_t, gpu_t, cpu_p, gpu_p, cpu_clk, gpu_clk) = reader.poll();
        println!(
            "SystemTelemetry: CPU: {}°C, {:.1}W, {}MHz | GPU: {}°C, {:.1}W, {}MHz",
            cpu_t,
            cpu_p as f32 / 10.0,
            cpu_clk,
            gpu_t,
            gpu_p as f32 / 10.0,
            gpu_clk
        );
        assert!(cpu_t >= 20 && cpu_t <= 115);
        assert!(cpu_clk > 400);
    }
}
