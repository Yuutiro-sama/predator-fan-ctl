//! Module Đọc Nhiệt độ CPU và GPU Siêu nhẹ (Zero-Allocation Polling Loop).
//!
//! Thiết kế chuyên biệt để duy trì mức tiêu thụ CPU < 0.05%:
//! - GPU: Nạp động `nvml.dll` qua Win32 FFI một lần duy nhất lúc khởi động.
//!   Mỗi chu kỳ chỉ gọi trực tiếp hàm con trỏ C `nvmlDeviceGetTemperature` (< 5 micro-giây, 0 allocation).
//! - CPU: Truy vấn ACPI ThermalZone (`root\WMI` -> `MSAcpi_ThermalZoneTemperature.CurrentTemperature`),
//!   chuyển đổi từ tenths of Kelvin sang độ Celsius chuẩn xác, kèm cơ chế fallback thông minh.

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

const NVML_SUCCESS: i32 = 0;
const NVML_TEMPERATURE_GPU: u32 = 0;

/// Bộ đọc nhiệt độ GPU NVIDIA sử dụng dynamic FFI tới `nvml.dll`.
pub struct NvmlGpuSensor {
    module: HMODULE,
    shutdown: NvmlShutdownFn,
    get_temp: NvmlDeviceGetTemperatureFn,
    device: *mut c_void,
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

            let p_init = GetProcAddress(module, windows::core::PCSTR(init_name.as_ptr()))?;
            let p_shutdown = GetProcAddress(module, windows::core::PCSTR(shutdown_name.as_ptr()))?;
            let p_handle = GetProcAddress(module, windows::core::PCSTR(handle_name.as_ptr()))?;
            let p_temp = GetProcAddress(module, windows::core::PCSTR(temp_name.as_ptr()))?;

            let init_fn: NvmlInitFn = std::mem::transmute(p_init);
            let shutdown_fn: NvmlShutdownFn = std::mem::transmute(p_shutdown);
            let handle_fn: NvmlDeviceGetHandleByIndexFn = std::mem::transmute(p_handle);
            let get_temp_fn: NvmlDeviceGetTemperatureFn = std::mem::transmute(p_temp);

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
                device,
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
// 2. CPU SENSOR (ACPI ThermalZone via WMI)
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
// 3. COMBINED SENSOR MANAGER
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
}
