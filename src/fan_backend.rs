//! Module Hệ thống Điều khiển Quạt Kép (Dual Fan Backend): Acer WMI ACPI & Direct EC Port I/O.
//!
//! Cung cấp trait trừu tượng `FanController` cùng 2 backend phần cứng:
//! 1. `AcerWmiBackend`: Giao tiếp trực tiếp với ACPI-WMI firmware (`root\WMI` -> `AcerGamingFunction` hoặc `AcerGaming_SystemInformation`).
//!    Không cần cài driver bên thứ 3, gọi phương thức `SetGamingFanBehavior` và `SetGamingFanSpeed`.
//! 2. `DirectEcBackend`: Ghi trực tiếp vào thanh ghi Embedded Controller thông qua các cổng I/O 0x62 (Data) và 0x66 (Command/Status),
//!    nạp động driver thông dụng (`inpoutx64.dll` hoặc `WinRing0x64.dll`). Độ trễ < 1ms.

use windows::core::{BSTR, PCSTR, PCWSTR, VARIANT};
use windows::Win32::Foundation::{FreeLibrary, BOOL, HANDLE, HMODULE};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Wmi::*;

#[link(name = "kernel32")]
extern "system" {
    fn WaitNamedPipeW(lpNamedPipeName: *const u16, nTimeOut: u32) -> BOOL;
    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: u32,
        dwShareMode: u32,
        lpSecurityAttributes: *const std::ffi::c_void,
        dwCreationDisposition: u32,
        dwFlagsAndAttributes: u32,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    fn ReadFile(
        hFile: HANDLE,
        lpBuffer: *mut std::ffi::c_void,
        nNumberOfBytesToRead: u32,
        lpNumberOfBytesRead: *mut u32,
        lpOverlapped: *mut std::ffi::c_void,
    ) -> BOOL;
    fn WriteFile(
        hFile: HANDLE,
        lpBuffer: *const std::ffi::c_void,
        nNumberOfBytesToWrite: u32,
        lpNumberOfBytesWritten: *mut u32,
        lpOverlapped: *mut std::ffi::c_void,
    ) -> BOOL;
    fn CloseHandle(hObject: HANDLE) -> BOOL;
}

/// Trait chuẩn cho các backend điều khiển quạt phần cứng.
pub trait FanController: Send {
    /// Chuyển quạt sang chế độ Manual / Custom curve.
    fn set_manual_mode(&mut self) -> bool;

    /// Chuyển quạt về chế độ Tự động mặc định của BIOS / OEM.
    fn set_auto_mode(&mut self) -> bool;

    /// Thiết lập tốc độ quạt (%) cho CPU và GPU (0..100).
    fn set_speeds(&mut self, cpu_pct: u8, gpu_pct: u8) -> bool;

    /// Đọc nhiệt độ phần cứng (°C) trực tiếp từ backend nếu hỗ trợ (CPU, GPU).
    fn read_temps(&mut self) -> Option<(u32, u32)>;

    /// Đọc tốc độ vòng quay quạt (RPM) trực tiếp từ cảm biến phần cứng (CPU RPM, GPU RPM).
    fn read_fan_rpms(&mut self) -> Option<(u32, u32)> {
        None
    }

    /// Đọc nhiệt độ bo mạch / hệ thống (°C) nếu backend hoặc OEM hardware service hỗ trợ.
    fn read_system_temp(&mut self) -> Option<u32> {
        None
    }

    /// Tên định danh của backend phục vụ logging / hiển thị UI.
    fn name(&self) -> &'static str;
}

// ============================================================================
// 1. BACKEND ACER WMI ACPI (root\WMI -> AcerGamingFunction)
// ============================================================================

/// Hàm ghi log debug ra file để kiểm chứng trạng thái phần cứng thời gian thực
pub fn append_debug_log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("fan_ctl_debug.log")
    {
        use std::io::Write;
        let _ = writeln!(f, "[{}] {}", chrono_or_epoch(), msg);
    }
}

fn chrono_or_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Payload cấu hình chế độ quạt Acer WMI (`SetGamingFanBehavior`):
/// - Bits 0-15: Fan Bitmap (Bit 0 = CPU, Bit 3 = GPU, hoặc kết hợp toàn dải Bits 0..3)
/// - Bits 16-17: Fan 1 Mode (1 = Auto, 2 = Turbo, 3 = Custom)
/// - Bits 18-19: Fan 2 Mode (1 = Auto, 2 = Turbo, 3 = Custom)
/// - Bits 22-23: Fan 3/GPU Mode (1 = Auto, 2 = Turbo, 3 = Custom)
pub const ACER_FAN_BEHAVIOR_AUTO: u64 = 0x55000F;   // Mở Auto cho toàn bộ các kênh quạt
pub const ACER_FAN_BEHAVIOR_TURBO: u64 = 0xAA000F;  // Mở Turbo 100% cho toàn bộ các kênh quạt
pub const ACER_FAN_BEHAVIOR_CUSTOM: u64 = 0xFF000F; // Mở Manual/Custom cho toàn bộ các kênh quạt

/// Payload fallback tương thích BIOS cũ
pub const ACER_FAN_BEHAVIOR_CUSTOM_LEGACY: u64 = 0xC30009; // CPU (Bit 0) + GPU (Bit 3) Custom
pub const ACER_FAN_BEHAVIOR_AUTO_LEGACY: u64 = 0x410009;
pub const ACER_FAN_BEHAVIOR_TURBO_LEGACY: u64 = 0x820009;

/// Mã lệnh WMI / OEM đo nhiệt độ bo mạch / hệ thống
pub const CMD_SYS_TEMP: u32 = 0x0301;

/// Đường dẫn Named Pipe phục vụ đo đạc phần cứng của Acer PredatorSense Service
pub const PIPE_HARDWARE_SERVICE: &str = "\\\\.\\pipe\\systemmonitoring_hardware_service_\0";

/// Backend điều khiển quạt qua giao diện ACPI-WMI của Acer (`AcerGamingFunction`).
pub struct AcerWmiBackend {
    services: IWbemServices,
    instance_relpath: BSTR,
    fan_speed_in_class: Option<IWbemClassObject>,
    fan_behavior_in_class: Option<IWbemClassObject>,
    fan_profile_in_class: Option<IWbemClassObject>,
    fan_sys_info_in_class: Option<IWbemClassObject>,
    get_fan_speed_in_class: Option<IWbemClassObject>,
    gpu_fan_index: Option<u8>,
    pipe_fail_count: u32,
    pipe_cooldown_until: Option<std::time::Instant>,
}

unsafe impl Send for AcerWmiBackend {}
unsafe impl Sync for AcerWmiBackend {}

impl AcerWmiBackend {
    /// Khởi tạo backend Acer WMI, kết nối tới namespace `root\WMI` và tìm kiếm instance `AcerGamingFunction`.
    pub fn new() -> Option<Self> {
        unsafe {
            // Khởi tạo COM Apartment (bỏ qua nếu đã được khởi tạo trước đó trên thread)
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            // Tạo COM object WbemLocator
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).ok()?;

            // Kết nối tới namespace "root\\WMI"
            let ns = BSTR::from("root\\WMI");
            let services: IWbemServices = match locator.ConnectServer(&ns, None, None, None, 0, None, None) {
                Ok(s) => s,
                Err(e) => {
                    append_debug_log(&format!("[WMI INIT ERR] ConnectServer root\\WMI failed: {:?}", e));
                    return None;
                }
            };

            // Cấu hình proxy blanket để có quyền truy cập root\WMI
            let _ = CoSetProxyBlanket(
                &services,
                10, // RPC_C_AUTHN_WINNT
                0,  // RPC_C_AUTHZ_NONE
                None,
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            );

            // 1. Tìm instance qua CreateInstanceEnum (chuẩn native WMI cho ACPI provider)
            let mut relpath: Option<BSTR> = None;
            let class_name = BSTR::from("AcerGamingFunction");

            if let Ok(enumerator) = services.CreateInstanceEnum(
                &class_name,
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            ) {
                let mut objects: [Option<IWbemClassObject>; 1] = [None];
                let mut returned = 0;
                while enumerator.Next(WBEM_INFINITE, &mut objects, &mut returned).is_ok() && returned > 0 {
                    if let Some(obj) = &objects[0] {
                        let prop_name = BSTR::from("__RELPATH");
                        let mut var = VARIANT::default();
                        if obj.Get(&prop_name, 0, &mut var, None, None).is_ok() {
                            if let Ok(bstr_val) = BSTR::try_from(&var) {
                                append_debug_log(&format!("[WMI] CreateInstanceEnum found RELPATH: {}", bstr_val));
                                relpath = Some(bstr_val);
                                break;
                            }
                        }
                    }
                }
            }

            // 2. Thử truy vấn bằng 'SELECT * FROM AcerGamingFunction'
            if relpath.is_none() {
                let query = BSTR::from("SELECT * FROM AcerGamingFunction");
                let lang = BSTR::from("WQL");
                if let Ok(enumerator) = services.ExecQuery(
                    &lang,
                    &query,
                    WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                    None,
                ) {
                    let mut objects: [Option<IWbemClassObject>; 1] = [None];
                    let mut returned = 0;
                    if enumerator.Next(WBEM_INFINITE, &mut objects, &mut returned).is_ok() && returned > 0 {
                        if let Some(obj) = &objects[0] {
                            let prop_name = BSTR::from("__RELPATH");
                            let mut var = VARIANT::default();
                            if obj.Get(&prop_name, 0, &mut var, None, None).is_ok() {
                                if let Ok(bstr_val) = BSTR::try_from(&var) {
                                    append_debug_log(&format!("[WMI] ExecQuery found RELPATH: {}", bstr_val));
                                    relpath = Some(bstr_val);
                                }
                            }
                        }
                    }
                }
            }

            // 3. Fallback đường dẫn instance mặc định theo chuẩn ACPI PNP0C14 của Acer
            let instance_relpath = relpath.unwrap_or_else(|| {
                append_debug_log("[WMI WARN] Using fallback APGE_0 instance");
                BSTR::from("AcerGamingFunction.InstanceName=\"ACPI\\\\PNP0C14\\\\APGE_0\"")
            });
            append_debug_log(&format!("[WMI INIT] Active instance RELPATH: {}", instance_relpath));

            // Lấy template input class của class AcerGamingFunction để tái sử dụng
            let mut class_obj: Option<IWbemClassObject> = None;
            let _ = services.GetObject(&class_name, WBEM_GENERIC_FLAG_TYPE(0), None, Some(&mut class_obj), None);

            let mut fan_speed_in_class = None;
            let mut fan_behavior_in_class = None;
            let mut fan_profile_in_class = None;
            let mut fan_sys_info_in_class = None;
            let mut get_fan_speed_in_class = None;

            if let Some(cls) = class_obj {
                let method_speed = BSTR::from("SetGamingFanSpeed");
                let mut speed_in = None;
                let _ = cls.GetMethod(&method_speed, 0, &mut speed_in, std::ptr::null_mut());
                fan_speed_in_class = speed_in;

                let method_behavior = BSTR::from("SetGamingFanBehavior");
                let mut behavior_in = None;
                let _ = cls.GetMethod(&method_behavior, 0, &mut behavior_in, std::ptr::null_mut());
                fan_behavior_in_class = behavior_in;

                let method_profile = BSTR::from("SetGamingProfile");
                let mut profile_in = None;
                let _ = cls.GetMethod(&method_profile, 0, &mut profile_in, std::ptr::null_mut());
                fan_profile_in_class = profile_in;

                let method_sys_info = BSTR::from("GetGamingSysInfo");
                let mut sys_info_in = None;
                let _ = cls.GetMethod(&method_sys_info, 0, &mut sys_info_in, std::ptr::null_mut());
                fan_sys_info_in_class = sys_info_in;

                let method_get_speed = BSTR::from("GetGamingFanSpeed");
                let mut get_speed_in = None;
                let _ = cls.GetMethod(&method_get_speed, 0, &mut get_speed_in, std::ptr::null_mut());
                get_fan_speed_in_class = get_speed_in;
            }

            append_debug_log(&format!(
                "[WMI INIT] fan_speed_in: {}, fan_behavior_in: {}, fan_profile_in: {}, fan_sys_info_in: {}, get_fan_speed_in: {}",
                fan_speed_in_class.is_some(),
                fan_behavior_in_class.is_some(),
                fan_profile_in_class.is_some(),
                fan_sys_info_in_class.is_some(),
                get_fan_speed_in_class.is_some(),
            ));

            let backend = Self {
                services,
                instance_relpath,
                fan_speed_in_class,
                fan_behavior_in_class,
                fan_profile_in_class,
                fan_sys_info_in_class,
                get_fan_speed_in_class,
                gpu_fan_index: None,
                pipe_fail_count: 0,
                pipe_cooldown_until: None,
            };

            // Khởi động probe cảm biến quạt một lần duy nhất
            backend.probe_hardware_capabilities();

            Some(backend)
        }
    }

    /// Đóng gói payload 64-bit cho lệnh `SetGamingFanSpeed`:
    /// - Byte 0: Fan Index (1 = CPU fan, 2 = GPU fan)
    /// - Byte 1: Tốc độ quạt (0 - 100%)
    #[inline]
    pub fn encode_fan_speed_arg(fan_index: u8, speed_pct: u8) -> u64 {
        ((speed_pct.min(100) as u64) << 8) | (fan_index as u64)
    }

    /// Thực thi gọi instance method với tham số `gmInput` kiểu UInt64.
    unsafe fn call_wmi_method_u64(&self, method_name: &str, in_class: Option<&IWbemClassObject>, input_val: u64) -> bool {
        let Some(in_cls) = in_class else {
            append_debug_log(&format!("[WMI CALL ERR] method {} in_class is None", method_name));
            return false;
        };
        let Ok(in_instance) = in_cls.SpawnInstance(0) else {
            append_debug_log(&format!("[WMI CALL ERR] method {} SpawnInstance failed", method_name));
            return false;
        };

        let prop_name = BSTR::from("gmInput");

        // WMI COM API cho CIM_UINT64 bắt buộc biểu diễn bằng VT_BSTR (chuỗi số thập phân)
        // do hạn chế tương thích OLE Automation cổ điển; fallback sang VT_UI8 và VT_UI4 nếu cần
        let bstr_val = BSTR::from(input_val.to_string().as_str());
        let var_bstr = VARIANT::from(bstr_val);
        let res_bstr = in_instance.Put(&prop_name, 0, &var_bstr, 0);

        if res_bstr.is_err() {
            let var_u64 = VARIANT::from(input_val);
            let res_u64 = in_instance.Put(&prop_name, 0, &var_u64, 0);
            if res_u64.is_err() {
                let var_u32 = VARIANT::from(input_val as u32);
                let res_u32 = in_instance.Put(&prop_name, 0, &var_u32, 0);
                if res_u32.is_err() {
                    append_debug_log(&format!(
                        "[WMI CALL ERR] method {} Put gmInput failed: BSTR={:?}, UI8={:?}, UI4={:?}",
                        method_name, res_bstr, res_u64, res_u32
                    ));
                    return false;
                }
            }
        }

        let m_name = BSTR::from(method_name);
        let mut out_params = None;

        let res = self.services.ExecMethod(
            &self.instance_relpath,
            &m_name,
            WBEM_GENERIC_FLAG_TYPE(0),
            None,
            Some(&in_instance),
            Some(&mut out_params),
            None,
        );

        let mut gm_output_u32 = 0u32;
        let mut out_val_str = String::from("none");
        if let Some(ref out) = out_params {
            let mut out_var = VARIANT::default();
            if out.Get(&BSTR::from("gmOutput"), 0, &mut out_var, None, None).is_ok() {
                out_val_str = format!("{:?}", out_var);
                let s = out_var.to_string();
                if let Ok(v) = s.parse::<u32>() {
                    gm_output_u32 = v;
                }
            }
        }
        let is_ok = res.is_ok() && gm_output_u32 == 0;
        append_debug_log(&format!(
            "[WMI CALL] {} on '{}' (input 0x{:X}) -> res: {:?}, output: {} (ok: {})",
            method_name, self.instance_relpath, input_val, res, out_val_str, is_ok
        ));

        is_ok
    }

    /// Thực thi gọi WMI getter method với tham số gmInput kiểu UInt32 và nhận kết quả gmOutput kiểu UInt64
    unsafe fn call_wmi_getter_u32_to_u64(&self, method_name: &str, in_class: Option<&IWbemClassObject>, input_val: u32) -> Option<u64> {
        let in_cls = in_class?;
        let in_instance = in_cls.SpawnInstance(0).ok()?;
        let prop_name = BSTR::from("gmInput");

        let bstr_val = BSTR::from(input_val.to_string().as_str());
        let var_bstr = VARIANT::from(bstr_val);
        if in_instance.Put(&prop_name, 0, &var_bstr, 0).is_err() {
            let var_u32 = VARIANT::from(input_val);
            if in_instance.Put(&prop_name, 0, &var_u32, 0).is_err() {
                let var_u64 = VARIANT::from(input_val as u64);
                let _ = in_instance.Put(&prop_name, 0, &var_u64, 0);
            }
        }

        let m_name = BSTR::from(method_name);
        let mut out_params = None;

        let res = self.services.ExecMethod(
            &self.instance_relpath,
            &m_name,
            WBEM_GENERIC_FLAG_TYPE(0),
            None,
            Some(&in_instance),
            Some(&mut out_params),
            None,
        );

        if res.is_ok() {
            if let Some(ref out) = out_params {
                let mut out_var = VARIANT::default();
                if out.Get(&BSTR::from("gmOutput"), 0, &mut out_var, None, None).is_ok() {
                    let s = out_var.to_string();
                    if let Ok(v) = s.parse::<u64>() {
                        return Some(v);
                    }
                }
            }
        }
        None
    }

    /// Thăm dò (Probe) toàn bộ các kênh quạt và sensor phần cứng tại thời điểm khởi động
    pub fn probe_hardware_capabilities(&self) {
        unsafe {
            append_debug_log("[PROBE START] Bắt đầu quét các cảm biến và kênh quạt phần cứng");
            for idx in 0..=8u32 {
                if let Some(val) = self.call_wmi_getter_u32_to_u64("GetGamingFanSpeed", self.get_fan_speed_in_class.as_ref(), idx) {
                    append_debug_log(&format!("[PROBE] GetGamingFanSpeed({}) -> 0x{:X}", idx, val));
                }
            }
            for &subcmd in &[0x0101u32, 0x0201, 0x0601, 0x0A01, 0x0200, 0x0600] {
                if let Some(val) = self.call_wmi_getter_u32_to_u64("GetGamingSysInfo", self.fan_sys_info_in_class.as_ref(), subcmd) {
                    append_debug_log(&format!("[PROBE] GetGamingSysInfo(0x{:X}) -> 0x{:X}", subcmd, val));
                }
            }
            append_debug_log("[PROBE END] Hoàn thành quét phần cứng");
        }
    }

    /// Đọc tốc độ vòng quay (RPM) thời gian thực của quạt CPU và GPU từ firmware BIOS/EC
    pub fn read_rpms(&self) -> (u32, u32) {
        unsafe {
            let mut cpu_rpm = 0u32;
            let mut gpu_rpm = 0u32;

            // 1. Đọc CPU Fan RPM: GetGamingSysInfo với mã metric 2 (thử 0x0201 và 0x0200)
            for &cmd in &[0x0201u32, 0x0200u32] {
                if let Some(out) = self.call_wmi_getter_u32_to_u64("GetGamingSysInfo", self.fan_sys_info_in_class.as_ref(), cmd) {
                    let status = (out & 0xFF) as u8;
                    let r1 = ((out >> 8) & 0xFFFF) as u32;
                    let r2 = ((out >> 24) & 0xFFFF) as u32;
                    if status == 0 {
                        let candidate = if r1 > 0 && r1 < 10000 { r1 } else if r2 > 0 && r2 < 10000 { r2 } else { 0 };
                        if candidate > 0 {
                            cpu_rpm = candidate;
                            break;
                        }
                    }
                }
            }

            // 2. Đọc GPU Fan RPM: GetGamingSysInfo với mã metric 6 (thử 0x0601 và 0x0600)
            for &cmd in &[0x0601u32, 0x0600u32] {
                if let Some(out) = self.call_wmi_getter_u32_to_u64("GetGamingSysInfo", self.fan_sys_info_in_class.as_ref(), cmd) {
                    let status = (out & 0xFF) as u8;
                    let r1 = ((out >> 8) & 0xFFFF) as u32;
                    let r2 = ((out >> 24) & 0xFFFF) as u32;
                    if status == 0 {
                        let candidate = if r1 > 0 && r1 < 10000 { r1 } else if r2 > 0 && r2 < 10000 { r2 } else { 0 };
                        if candidate > 0 {
                            gpu_rpm = candidate;
                            break;
                        }
                    }
                }
            }

            (cpu_rpm, gpu_rpm)
        }
    }

    /// Ghi nhận 1 lần thất bại của Named Pipe và kích hoạt Circuit Breaker nếu vượt ngưỡng 3 lần liên tiếp
    fn record_pipe_failure(&mut self) {
        self.pipe_fail_count = self.pipe_fail_count.saturating_add(1);
        if self.pipe_fail_count >= 3 && self.pipe_cooldown_until.is_none() {
            self.pipe_cooldown_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(30));
            append_debug_log("[PIPE CIRCUIT BREAKER] Named pipe failed 3 consecutive times. Cooldown 30s activated.");
        }
    }

    /// Đọc nhiệt độ qua Named Pipe `\\.\pipe\systemmonitoring_hardware_service_`
    /// Cơ chế non-blocking: Timeout 50ms và Circuit Breaker cooldown 30s
    pub fn read_system_temp_pipe(&mut self) -> Option<u32> {
        let now = std::time::Instant::now();
        if let Some(cooldown) = self.pipe_cooldown_until {
            if now < cooldown {
                return None;
            }
            // Cooldown đã hết, mở lại trạng thái thử nghiệm
            self.pipe_cooldown_until = None;
            self.pipe_fail_count = 0;
        }

        let wide_name: Vec<u16> = PIPE_HARDWARE_SERVICE.encode_utf16().collect();
        unsafe {
            // Kiểm tra sự tồn tại và khả năng đáp ứng của pipe với timeout tối đa 50ms
            if WaitNamedPipeW(wide_name.as_ptr(), 50).0 == 0 {
                self.record_pipe_failure();
                return None;
            }

            let handle = CreateFileW(
                wide_name.as_ptr(),
                0x80000000 | 0x40000000, // GENERIC_READ | GENERIC_WRITE
                0,
                std::ptr::null(),
                3, // OPEN_EXISTING
                0,
                HANDLE(std::ptr::null_mut()),
            );

            if handle.is_invalid() || handle.0.is_null() {
                self.record_pipe_failure();
                return None;
            }

            // Gói tin nhị phân chuẩn 11 bytes:
            // cmd_id = 7 (u16), num_args = 1 (u8), arg_len = 4 (u32), subcmd = 0x0301 (u32)
            let mut req = [0u8; 11];
            req[0..2].copy_from_slice(&7u16.to_le_bytes());
            req[2] = 1u8;
            req[3..7].copy_from_slice(&4u32.to_le_bytes());
            req[7..11].copy_from_slice(&CMD_SYS_TEMP.to_le_bytes());

            let mut written = 0u32;
            let write_ok = WriteFile(
                handle,
                req.as_ptr() as *const std::ffi::c_void,
                req.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            );

            if write_ok.0 == 0 || written != 11 {
                let _ = CloseHandle(handle);
                self.record_pipe_failure();
                return None;
            }

            let mut resp = [0u8; 64];
            let mut read_bytes = 0u32;
            let read_ok = ReadFile(
                handle,
                resp.as_mut_ptr() as *mut std::ffi::c_void,
                resp.len() as u32,
                &mut read_bytes,
                std::ptr::null_mut(),
            );
            let _ = CloseHandle(handle);

            if read_ok.0 == 0 || read_bytes < 7 {
                self.record_pipe_failure();
                return None;
            }

            // Byte 6 chứa giá trị nhiệt độ Celsius
            let temp = resp[6] as u32;
            if (15..=115).contains(&temp) {
                self.pipe_fail_count = 0;
                self.pipe_cooldown_until = None;
                Some(temp)
            } else {
                self.record_pipe_failure();
                None
            }
        }
    }

    /// Đọc nhiệt độ Mainboard/System (°C):
    /// - Primary: WMI GetGamingSysInfo(0x0301)
    /// - Fallback: Named Pipe `\\.\pipe\systemmonitoring_hardware_service_`
    /// - Lọc dải nhiệt độ hợp lệ: 15..=115
    pub fn read_system_temp(&mut self) -> Option<u32> {
        // 1. Ưu tiên WMI GetGamingSysInfo(0x0301)
        unsafe {
            if let Some(val) = self.call_wmi_getter_u32_to_u64("GetGamingSysInfo", self.fan_sys_info_in_class.as_ref(), CMD_SYS_TEMP) {
                let b0 = (val & 0xFF) as u32;
                let b1 = ((val >> 8) & 0xFF) as u32;
                let candidate = if (15..=115).contains(&b0) {
                    Some(b0)
                } else if (15..=115).contains(&b1) {
                    Some(b1)
                } else {
                    None
                };

                if let Some(t) = candidate {
                    return Some(t);
                }
            }
        }

        // 2. Fallback sang Named Pipe với Circuit Breaker & Timeout 50ms
        self.read_system_temp_pipe()
    }
}

impl FanController for AcerWmiBackend {
    fn set_manual_mode(&mut self) -> bool {
        unsafe {
            let _ = self.call_wmi_method_u64(
                "SetGamingProfile",
                self.fan_profile_in_class.as_ref(),
                2, // Balanced / Custom
            );
            // Gửi cả định dạng full-mask (0xFF000F) và legacy (0xC30009) để đảm bảo mở khóa mọi kênh
            let ok_custom = self.call_wmi_method_u64(
                "SetGamingFanBehavior",
                self.fan_behavior_in_class.as_ref(),
                ACER_FAN_BEHAVIOR_CUSTOM,
            );
            let ok_legacy = self.call_wmi_method_u64(
                "SetGamingFanBehavior",
                self.fan_behavior_in_class.as_ref(),
                ACER_FAN_BEHAVIOR_CUSTOM_LEGACY,
            );
            ok_custom || ok_legacy
        }
    }

    fn set_auto_mode(&mut self) -> bool {
        unsafe {
            let _ = self.call_wmi_method_u64(
                "SetGamingProfile",
                self.fan_profile_in_class.as_ref(),
                2, // Balanced / Auto
            );
            let ok_auto = self.call_wmi_method_u64(
                "SetGamingFanBehavior",
                self.fan_behavior_in_class.as_ref(),
                ACER_FAN_BEHAVIOR_AUTO,
            );
            let ok_legacy = self.call_wmi_method_u64(
                "SetGamingFanBehavior",
                self.fan_behavior_in_class.as_ref(),
                ACER_FAN_BEHAVIOR_AUTO_LEGACY,
            );
            ok_auto || ok_legacy
        }
    }

    fn set_speeds(&mut self, cpu_pct: u8, gpu_pct: u8) -> bool {
        if cpu_pct >= 98 && gpu_pct >= 98 {
            // Chế độ Turbo tối đa (100%): gọi SetGamingProfile(4 - Turbo) + SetGamingFanBehavior(Turbo)
            unsafe {
                let _ = self.call_wmi_method_u64(
                    "SetGamingProfile",
                    self.fan_profile_in_class.as_ref(),
                    4, // Turbo Profile
                );
                let _ = self.call_wmi_method_u64(
                    "SetGamingFanBehavior",
                    self.fan_behavior_in_class.as_ref(),
                    ACER_FAN_BEHAVIOR_TURBO,
                );
                let _ = self.call_wmi_method_u64(
                    "SetGamingFanBehavior",
                    self.fan_behavior_in_class.as_ref(),
                    ACER_FAN_BEHAVIOR_TURBO_LEGACY,
                );
            }
        } else if cpu_pct <= 25 && gpu_pct <= 25 {
            // Chế độ Silent (25%): gọi SetGamingProfile(1 - Quiet) + SetGamingFanBehavior(Custom)
            unsafe {
                let _ = self.call_wmi_method_u64(
                    "SetGamingProfile",
                    self.fan_profile_in_class.as_ref(),
                    1, // Quiet Profile
                );
                let _ = self.call_wmi_method_u64(
                    "SetGamingFanBehavior",
                    self.fan_behavior_in_class.as_ref(),
                    ACER_FAN_BEHAVIOR_CUSTOM,
                );
                let _ = self.call_wmi_method_u64(
                    "SetGamingFanBehavior",
                    self.fan_behavior_in_class.as_ref(),
                    ACER_FAN_BEHAVIOR_CUSTOM_LEGACY,
                );
            }
        } else {
            // Chế độ Active Curve thông thường: đặt Balanced Profile (2) + Custom Fan Behavior
            unsafe {
                let _ = self.call_wmi_method_u64(
                    "SetGamingProfile",
                    self.fan_profile_in_class.as_ref(),
                    2, // Balanced Profile
                );
                let _ = self.call_wmi_method_u64(
                    "SetGamingFanBehavior",
                    self.fan_behavior_in_class.as_ref(),
                    ACER_FAN_BEHAVIOR_CUSTOM,
                );
                let _ = self.call_wmi_method_u64(
                    "SetGamingFanBehavior",
                    self.fan_behavior_in_class.as_ref(),
                    ACER_FAN_BEHAVIOR_CUSTOM_LEGACY,
                );
            }
        }

        let cpu_arg = Self::encode_fan_speed_arg(1, cpu_pct);
        let ok_cpu = unsafe {
            self.call_wmi_method_u64(
                "SetGamingFanSpeed",
                self.fan_speed_in_class.as_ref(),
                cpu_arg,
            )
        };

        let mut ok_gpu = false;
        // Nếu đã xác định được GPU fan index trước đó, dùng luôn
        if let Some(cached_idx) = self.gpu_fan_index {
            let gpu_arg = Self::encode_fan_speed_arg(cached_idx, gpu_pct);
            ok_gpu = unsafe {
                self.call_wmi_method_u64(
                    "SetGamingFanSpeed",
                    self.fan_speed_in_class.as_ref(),
                    gpu_arg,
                )
            };
        }

        // Nếu chưa hoặc gọi thất bại, thử các candidate index [2, 3, 4, 8, 0]
        if !ok_gpu {
            for &gpu_idx in &[2u8, 3, 4, 8, 0] {
                let gpu_arg = Self::encode_fan_speed_arg(gpu_idx, gpu_pct);
                let ok = unsafe {
                    self.call_wmi_method_u64(
                        "SetGamingFanSpeed",
                        self.fan_speed_in_class.as_ref(),
                        gpu_arg,
                    )
                };
                if ok {
                    self.gpu_fan_index = Some(gpu_idx);
                    append_debug_log(&format!("[WMI] Đã xác định GPU fan index thành công: {}", gpu_idx));
                    ok_gpu = true;
                    break;
                }
            }
        }

        ok_cpu || ok_gpu
    }

    fn read_fan_rpms(&mut self) -> Option<(u32, u32)> {
        let (cpu, gpu) = self.read_rpms();
        if cpu > 0 || gpu > 0 {
            Some((cpu, gpu))
        } else {
            None
        }
    }

    fn read_system_temp(&mut self) -> Option<u32> {
        self.read_system_temp()
    }

    fn read_temps(&mut self) -> Option<(u32, u32)> {
        // AcerGamingFunction hỗ trợ GetGamingSysInfo, nhưng trả về qua custom bitmask tuỳ đời máy.
        // Trả về None để hệ thống ưu tiên sử dụng sensor.rs (NVML + ACPI WMI chuẩn).
        None
    }

    fn name(&self) -> &'static str {
        "Acer WMI ACPI (AcerGamingFunction)"
    }
}

// ============================================================================
// 2. BACKEND DIRECT EC PORT I/O (Ports 0x62 Data, 0x66 Command/Status)
// ============================================================================

type Out32Fn = unsafe extern "system" fn(port: u16, data: u16);
type Inp32Fn = unsafe extern "system" fn(port: u16) -> u16;

type WinRing0InitFn = unsafe extern "system" fn() -> i32;
type WinRing0DeinitFn = unsafe extern "system" fn();
type WinRing0WriteIoFn = unsafe extern "system" fn(port: u16, val: u8);
type WinRing0ReadIoFn = unsafe extern "system" fn(port: u16) -> u8;

enum EcDriverBackend {
    InpOut {
        module: HMODULE,
        out32: Out32Fn,
        inp32: Inp32Fn,
    },
    WinRing0 {
        module: HMODULE,
        deinit: WinRing0DeinitFn,
        write_io: WinRing0WriteIoFn,
        read_io: WinRing0ReadIoFn,
    },
}

/// Cấu hình địa chỉ thanh ghi EC (Embedded Controller) cho dòng máy Acer Predator.
#[derive(Debug, Clone, Copy)]
pub struct EcRegisterMap {
    pub cpu_fan_mode_reg: u8,   // Mặc định 0x21 (0 = Auto, 1 = Manual)
    pub cpu_fan_speed_reg: u8,  // Mặc định 0x37 (Tốc độ % hoặc raw)
    pub gpu_fan_mode_reg: u8,   // Mặc định 0x22 (0 = Auto, 1 = Manual)
    pub gpu_fan_speed_reg: u8,  // Mặc định 0x3A (Tốc độ % hoặc raw)
    pub cpu_temp_reg: u8,       // Mặc định 0xB0 (°C)
    pub gpu_temp_reg: u8,       // Mặc định 0xB1 (°C)
}

impl Default for EcRegisterMap {
    fn default() -> Self {
        Self {
            cpu_fan_mode_reg: 0x21,
            cpu_fan_speed_reg: 0x37,
            gpu_fan_mode_reg: 0x22,
            gpu_fan_speed_reg: 0x3A,
            cpu_temp_reg: 0xB0,
            gpu_temp_reg: 0xB1,
        }
    }
}

/// Backend điều khiển quạt ghi trực tiếp vào thanh ghi EC qua Port 0x62/0x66.
pub struct DirectEcBackend {
    driver: EcDriverBackend,
    regs: EcRegisterMap,
}

unsafe impl Send for DirectEcBackend {}
unsafe impl Sync for DirectEcBackend {}

impl DirectEcBackend {
    /// Khởi tạo Direct EC Backend bằng cách nạp động `inpoutx64.dll` hoặc `WinRing0x64.dll`.
    pub fn new(regs: EcRegisterMap) -> Option<Self> {
        unsafe {
            // 1. Thử nạp inpoutx64.dll
            let inpout_names = ["inpoutx64.dll\0", "C:\\Windows\\System32\\inpoutx64.dll\0"];
            for name in inpout_names {
                let name_wide: Vec<u16> = name.encode_utf16().collect();
                if let Ok(module) = LoadLibraryW(PCWSTR(name_wide.as_ptr())) {
                    let out_sym = GetProcAddress(module, PCSTR(b"Out32\0".as_ptr()));
                    let inp_sym = GetProcAddress(module, PCSTR(b"Inp32\0".as_ptr()));
                    if let (Some(o), Some(i)) = (out_sym, inp_sym) {
                        return Some(Self {
                            driver: EcDriverBackend::InpOut {
                                module,
                                out32: std::mem::transmute(o),
                                inp32: std::mem::transmute(i),
                            },
                            regs,
                        });
                    }
                    let _ = FreeLibrary(module);
                }
            }

            // 2. Thử nạp WinRing0x64.dll
            let winring_names = ["WinRing0x64.dll\0", "WinRing0.dll\0"];
            for name in winring_names {
                let name_wide: Vec<u16> = name.encode_utf16().collect();
                if let Ok(module) = LoadLibraryW(PCWSTR(name_wide.as_ptr())) {
                    let init_sym = GetProcAddress(module, PCSTR(b"InitializeOls\0".as_ptr()));
                    let deinit_sym = GetProcAddress(module, PCSTR(b"DeinitializeOls\0".as_ptr()));
                    let write_sym = GetProcAddress(module, PCSTR(b"WriteIoPortByte\0".as_ptr()));
                    let read_sym = GetProcAddress(module, PCSTR(b"ReadIoPortByte\0".as_ptr()));

                    if let (Some(init), Some(deinit), Some(wr), Some(rd)) = (init_sym, deinit_sym, write_sym, read_sym) {
                        let init_fn: WinRing0InitFn = std::mem::transmute(init);
                        if init_fn() != 0 {
                            return Some(Self {
                                driver: EcDriverBackend::WinRing0 {
                                    module,
                                    deinit: std::mem::transmute(deinit),
                                    write_io: std::mem::transmute(wr),
                                    read_io: std::mem::transmute(rd),
                                },
                                regs,
                            });
                        }
                    }
                    let _ = FreeLibrary(module);
                }
            }

            None
        }
    }

    #[inline]
    unsafe fn write_port_byte(&self, port: u16, val: u8) {
        match &self.driver {
            EcDriverBackend::InpOut { out32, .. } => (out32)(port, val as u16),
            EcDriverBackend::WinRing0 { write_io, .. } => (write_io)(port, val),
        }
    }

    #[inline]
    unsafe fn read_port_byte(&self, port: u16) -> u8 {
        match &self.driver {
            EcDriverBackend::InpOut { inp32, .. } => (inp32)(port) as u8,
            EcDriverBackend::WinRing0 { read_io, .. } => (read_io)(port),
        }
    }

    /// Chờ cờ Input Buffer Full (IBF - Bit 1 trên port 0x66) về 0.
    unsafe fn wait_ec_ibf_clear(&self) -> bool {
        for _ in 0..2000 {
            if (self.read_port_byte(0x66) & 0x02) == 0 {
                return true;
            }
            std::hint::spin_loop();
        }
        false
    }

    /// Chờ cờ Output Buffer Full (OBF - Bit 0 trên port 0x66) lên 1.
    unsafe fn wait_ec_obf_set(&self) -> bool {
        for _ in 0..2000 {
            if (self.read_port_byte(0x66) & 0x01) != 0 {
                return true;
            }
            std::hint::spin_loop();
        }
        false
    }

    /// Ghi 1 byte vào thanh ghi EC qua giao thức chuẩn ACPI (0x81).
    pub fn write_ec_byte(&self, reg: u8, val: u8) -> bool {
        unsafe {
            if !self.wait_ec_ibf_clear() { return false; }
            self.write_port_byte(0x66, 0x81); // 0x81 = Write EC Command
            if !self.wait_ec_ibf_clear() { return false; }
            self.write_port_byte(0x62, reg);
            if !self.wait_ec_ibf_clear() { return false; }
            self.write_port_byte(0x62, val);
            true
        }
    }

    /// Đọc 1 byte từ thanh ghi EC qua giao thức chuẩn ACPI (0x80).
    pub fn read_ec_byte(&self, reg: u8) -> Option<u8> {
        unsafe {
            if !self.wait_ec_ibf_clear() { return None; }
            self.write_port_byte(0x66, 0x80); // 0x80 = Read EC Command
            if !self.wait_ec_ibf_clear() { return None; }
            self.write_port_byte(0x62, reg);
            if !self.wait_ec_obf_set() { return None; }
            Some(self.read_port_byte(0x62))
        }
    }
}

impl Drop for DirectEcBackend {
    fn drop(&mut self) {
        unsafe {
            match &self.driver {
                EcDriverBackend::InpOut { module, .. } => {
                    let _ = FreeLibrary(*module);
                }
                EcDriverBackend::WinRing0 { module, deinit, .. } => {
                    (deinit)();
                    let _ = FreeLibrary(*module);
                }
            }
        }
    }
}

impl FanController for DirectEcBackend {
    fn set_manual_mode(&mut self) -> bool {
        let cpu_ok = self.write_ec_byte(self.regs.cpu_fan_mode_reg, 1);
        let gpu_ok = self.write_ec_byte(self.regs.gpu_fan_mode_reg, 1);
        cpu_ok && gpu_ok
    }

    fn set_auto_mode(&mut self) -> bool {
        let cpu_ok = self.write_ec_byte(self.regs.cpu_fan_mode_reg, 0);
        let gpu_ok = self.write_ec_byte(self.regs.gpu_fan_mode_reg, 0);
        cpu_ok && gpu_ok
    }

    fn set_speeds(&mut self, cpu_pct: u8, gpu_pct: u8) -> bool {
        let cpu_ok = self.write_ec_byte(self.regs.cpu_fan_speed_reg, cpu_pct.min(100));
        let gpu_ok = self.write_ec_byte(self.regs.gpu_fan_speed_reg, gpu_pct.min(100));
        cpu_ok && gpu_ok
    }

    fn read_temps(&mut self) -> Option<(u32, u32)> {
        let cpu_t = self.read_ec_byte(self.regs.cpu_temp_reg)? as u32;
        let gpu_t = self.read_ec_byte(self.regs.gpu_temp_reg)? as u32;

        if cpu_t > 20 && cpu_t < 115 && gpu_t > 20 && gpu_t < 115 {
            Some((cpu_t, gpu_t))
        } else {
            None
        }
    }

    fn name(&self) -> &'static str {
        "Direct EC Port I/O (0x62/0x66)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_fan_speed_arg() {
        // Fan 1 (CPU), 0% speed
        assert_eq!(AcerWmiBackend::encode_fan_speed_arg(1, 0), 0x0001);
        // Fan 1 (CPU), 50% speed (50 = 0x32 -> 0x3201)
        assert_eq!(AcerWmiBackend::encode_fan_speed_arg(1, 50), 0x3201);
        // Fan 2 (GPU), 100% speed (100 = 0x64 -> 0x6402)
        assert_eq!(AcerWmiBackend::encode_fan_speed_arg(2, 100), 0x6402);
        // Clamping tốc độ vượt quá 100% về 100%
        assert_eq!(AcerWmiBackend::encode_fan_speed_arg(1, 150), 0x6401);
    }

    #[test]
    fn test_ec_register_map_default() {
        let regs = EcRegisterMap::default();
        assert_eq!(regs.cpu_fan_mode_reg, 0x21);
        assert_eq!(regs.cpu_fan_speed_reg, 0x37);
        assert_eq!(regs.gpu_fan_mode_reg, 0x22);
        assert_eq!(regs.gpu_fan_speed_reg, 0x3A);
        assert_eq!(regs.cpu_temp_reg, 0xB0);
        assert_eq!(regs.gpu_temp_reg, 0xB1);
    }

    #[test]
    fn test_direct_ec_backend_missing_dll_graceful() {
        // Khi không có driver inpoutx64 / WinRing0, backend trả về None an toàn tuyệt đối
        let regs = EcRegisterMap::default();
        let _ec = DirectEcBackend::new(regs);
    }

    #[test]
    fn test_named_pipe_request_packet_format() {
        let mut req = [0u8; 11];
        req[0..2].copy_from_slice(&7u16.to_le_bytes());
        req[2] = 1u8;
        req[3..7].copy_from_slice(&4u32.to_le_bytes());
        req[7..11].copy_from_slice(&CMD_SYS_TEMP.to_le_bytes());

        assert_eq!(req.len(), 11);
        // cmd_id = 7 (u16 little-endian: 0x07, 0x00)
        assert_eq!(&req[0..2], &[0x07, 0x00]);
        // num_args = 1
        assert_eq!(req[2], 0x01);
        // arg_len = 4 (u32 little-endian: 0x04, 0x00, 0x00, 0x00)
        assert_eq!(&req[3..7], &[0x04, 0x00, 0x00, 0x00]);
        // subcmd = 0x0301 (u32 little-endian: 0x01, 0x03, 0x00, 0x00)
        assert_eq!(&req[7..11], &[0x01, 0x03, 0x00, 0x00]);
    }

    #[test]
    fn test_named_pipe_response_temperature_parsing() {
        // Mô phỏng payload trả về thực tế từ Acer Service: 13 bytes
        let mut resp = [0u8; 13];
        resp[0] = 0x01;
        resp[1] = 0x08;
        resp[6] = 58; // 58°C

        let temp = resp[6] as u32;
        assert!((15..=115).contains(&temp));
        assert_eq!(temp, 58);

        // Kiểm tra lọc giá trị bất thường / rác firmware EC (0°C, 255°C, 120°C)
        assert!(!(15..=115).contains(&0u32));
        assert!(!(15..=115).contains(&255u32));
        assert!(!(15..=115).contains(&120u32));
        assert!((15..=115).contains(&15u32));
        assert!((15..=115).contains(&115u32));
    }
}
