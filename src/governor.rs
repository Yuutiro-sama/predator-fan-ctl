//! Module Vi điều khiển Năng lượng & Nhiệt độ Động (Closed-Loop Thermal Governor).
//!
//! Quản lý và điều phối mượt mà trạng thái năng lượng vi xử lý thông qua
//! Intel Speed Shift Energy Performance Preference (`PERFEPP`) kết hợp cơ chế
//! **Emergency Turbo Cut via PROCTHROTTLEMAX** có tính trễ nhiệt (Hysteresis),
//! giúp dập tắt triệt để hiện tượng xung công suất đột ngột lên 124.6W và đỉnh nhiệt 96°C
//! trên vi xử lý Intel Core i7-13650HX (Raptor Lake-HX), đưa nhiệt độ về ổn định quanh mốc mục tiêu.

use serde::{Deserialize, Serialize};
use windows::core::GUID;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::System::Power::{
    PowerGetActiveScheme, PowerReadACValueIndex, PowerSetActiveScheme, PowerWriteACValueIndex,
    PowerWriteDCValueIndex,
};

/// GUID phân nhóm thiết lập quản lý năng lượng bộ vi xử lý (Processor Power Management Subgroup)
/// Value: 54533251-82be-4824-96c1-47b60b740d00
pub const GUID_PROCESSOR_SETTINGS_SUBGROUP: GUID = GUID {
    data1: 0x54533251,
    data2: 0x82be,
    data3: 0x4824,
    data4: [0x96, 0xc1, 0x47, 0xb6, 0x0b, 0x74, 0x0d, 0x00],
};

/// GUID thiết lập phần trăm xung nhịp CPU tối đa (Maximum Processor State / PROCTHROTTLEMAX)
/// Value: bc5038f7-23e0-4960-96da-33abaf5935ec
/// - 100%: Cho phép vi xử lý bung xung Turbo Boost tối đa (lên tới 4.9 GHz).
/// -  99%: Ngắt khẩn cấp Turbo Boost trên Windows, khóa CPU tại xung cơ bản (Base Clock 2.6 GHz),
///         hạ tức thì công suất gói từ 124.6W về ~25W - 35W để dập tắt spike nhiệt.
pub const GUID_PROCESSOR_THROTTLE_MAXIMUM: GUID = GUID {
    data1: 0xbc5038f7,
    data2: 0x23e0,
    data3: 0x4960,
    data4: [0x96, 0xda, 0x33, 0xab, 0xaf, 0x59, 0x35, 0xec],
};

/// GUID thiết lập thiên hướng hiệu năng / năng lượng (Energy Performance Preference / PERFEPP)
/// Value: 36687f9e-e3a5-4dbf-b1dc-15eb381c6863
/// Dải giá trị:
///   0: Maximum Performance (Speed Shift duy trì xung nhịp cao nhất có thể)
///  30: High Performance Smooth (~4.0 - 4.5 GHz, mát mẻ khi cách xa mục tiêu)
///  50: Windows Balanced mặc định (~3.4 - 3.8 GHz)
///  75: Throttled Smooth (tiệm cận ngưỡng an toàn target - 4°C)
///  90: Maximum Throttle Bias kết hợp cùng ngắt Turbo
pub const GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE: GUID = GUID {
    data1: 0x36687f9e,
    data2: 0xe3a5,
    data3: 0x4dbf,
    data4: [0xb1, 0xdc, 0x15, 0xeb, 0x38, 0x1c, 0x68, 0x63],
};

pub const fn default_governor_enabled() -> bool {
    true
}

pub const fn default_target_temp() -> u32 {
    80
}

pub const fn default_tolerance() -> u32 {
    0
}

pub const fn default_max_epp() -> u32 {
    90
}

/// Cấu hình tham số điều khiển bộ điều phối nhiệt độ CPU (Thermal Governor Configuration).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThermalGovernorConfig {
    /// Bật / tắt bộ điều phối nhiệt độ động
    #[serde(default = "default_governor_enabled")]
    pub enabled: bool,

    /// Nhiệt độ mục tiêu (°C) - mặc định 80°C (hoặc 75°C khi siết chặt)
    #[serde(default = "default_target_temp", alias = "target_cpu_temp")]
    pub target_temp: u32,

    /// Dải dung sai ổn định nhiệt độ (±°C)
    #[serde(default = "default_tolerance")]
    pub tolerance: u32,

    /// Giới hạn EPP tối đa - mặc định 90 để đủ lực hãm khi tải nặng cả CPU và GPU
    #[serde(default = "default_max_epp")]
    pub max_epp: u32,
}

impl Default for ThermalGovernorConfig {
    fn default() -> Self {
        Self {
            enabled: default_governor_enabled(),
            target_temp: default_target_temp(),
            tolerance: default_tolerance(),
            max_epp: default_max_epp(),
        }
    }
}

impl From<crate::config::ThermalGovernorConfig> for ThermalGovernorConfig {
    fn from(cfg: crate::config::ThermalGovernorConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            target_temp: cfg.target_cpu_temp,
            tolerance: cfg.tolerance,
            max_epp: cfg.max_epp,
        }
    }
}

impl From<&crate::config::ThermalGovernorConfig> for ThermalGovernorConfig {
    fn from(cfg: &crate::config::ThermalGovernorConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            target_temp: cfg.target_cpu_temp,
            tolerance: cfg.tolerance,
            max_epp: cfg.max_epp,
        }
    }
}

/// Bộ điều khiển nhiệt độ CPU động vòng kín kết hợp Emergency Turbo Cut (PROCTHROTTLEMAX 99%)
/// và Intel Speed Shift EPP tuyến tính (Smooth EPP Thermal Governor).
pub struct ThermalGovernor {
    /// Cấu hình hoạt động
    pub config: ThermalGovernorConfig,

    /// Mức Throttle tối đa hiện tại (100 = Turbo Boost bật, 99 = Turbo Cut)
    pub current_throttle_max: u32,

    /// Mức EPP hiện tại đang áp dụng (0..90)
    pub current_epp: u32,

    /// Nhiệt độ CPU chu kỳ trước để theo dõi tốc độ tăng nhiệt (dT/dt)
    pub prev_cpu_temp: Option<u32>,

    /// Mức EPP gốc đọc từ hệ điều hành lúc khởi động để khôi phục (hoặc 50)
    pub original_epp: u32,

    /// Mức Throttle tối đa cuối cùng đã ghi thành công vào Windows Power Subsystem
    pub last_applied_throttle_max: u32,

    /// Mức EPP cuối cùng đã ghi thành công vào Windows Power Subsystem
    pub last_applied_epp: u32,

    /// GUID của Power Scheme đang hoạt động trên Windows
    pub active_scheme_guid: Option<GUID>,

    /// Cờ kiểm soát việc ghi trực tiếp vào phần cứng (mặc định: true, dùng false trong unit test)
    pub apply_to_hardware: bool,
}

impl ThermalGovernor {
    /// Khởi tạo ThermalGovernor với cấu hình chỉ định.
    /// Tự động kết nối với Win32 Power Management subsystem, đảm bảo PROCTHROTTLEMAX = 100%,
    /// và đặt EPP ban đầu về 0 (Maximum Performance / Boost).
    pub fn new(config: ThermalGovernorConfig) -> Self {
        let (active_scheme_guid, original_epp) = Self::init_power_scheme();

        let mut governor = Self {
            config,
            current_throttle_max: 100,
            current_epp: 0,
            prev_cpu_temp: None,
            original_epp,
            last_applied_throttle_max: u32::MAX,
            last_applied_epp: u32::MAX,
            active_scheme_guid,
            apply_to_hardware: true,
        };

        // Ghi áp dụng trạng thái ban đầu (100% Throttle, EPP = 0)
        governor.apply_power_settings(100, 0);
        governor
    }

    /// Khởi tạo phiên bản kiểm thử không can thiệp trực tiếp vào thiết lập nguồn của hệ điều hành.
    pub fn new_test(config: ThermalGovernorConfig) -> Self {
        Self {
            config,
            current_throttle_max: 100,
            current_epp: 0,
            prev_cpu_temp: None,
            original_epp: 50,
            last_applied_throttle_max: 100,
            last_applied_epp: 0,
            active_scheme_guid: None,
            apply_to_hardware: false,
        }
    }

    /// Kiểm tra xem cơ chế Emergency Turbo Cut có đang được kích hoạt hay không.
    pub fn is_turbo_cut(&self) -> bool {
        self.current_throttle_max < 100
    }

    /// Hàm nội bộ đọc Active Power Scheme GUID, ghi đè PROCTHROTTLEMAX = 100% và đọc EPP gốc từ Windows
    fn init_power_scheme() -> (Option<GUID>, u32) {
        unsafe {
            let mut p_guid: *mut GUID = std::ptr::null_mut();
            let res = PowerGetActiveScheme(None, &mut p_guid);
            if res.0 != 0 || p_guid.is_null() {
                return (None, 50);
            }

            let scheme = *p_guid;
            let _ = LocalFree(HLOCAL(p_guid as _));

            // Đọc giá trị EPP hiện thời trên hệ thống
            let mut orig_val = 0u32;
            let read_res = PowerReadACValueIndex(
                None,
                Some(&scheme),
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE),
                &mut orig_val,
            );
            let orig = if read_res.0 == 0 && orig_val > 0 { orig_val } else { 50 };

            // Khởi tạo đảm bảo PROCTHROTTLEMAX = 100% để giữ Turbo Boost
            let _ = PowerWriteACValueIndex(
                None,
                &scheme,
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PROCESSOR_THROTTLE_MAXIMUM),
                100,
            );
            let _ = PowerWriteDCValueIndex(
                None,
                &scheme,
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PROCESSOR_THROTTLE_MAXIMUM),
                100,
            );
            let _ = PowerSetActiveScheme(None, Some(&scheme));

            (Some(scheme), orig)
        }
    }

    /// Tính toán mức EPP tuyến tính trong dải an toàn (Safe Zone):
    /// - Khi cách xa mục tiêu (dist = target - t_now >= 12°C): EPP = 30
    /// - Khi tiệm cận ngưỡng an toàn (dist = 4°C, tức target - 4°C): EPP = 75
    /// - Khi 4 < dist < 12: nội suy tuyến tính từ 30 lên 75
    /// - Khi dist < 4 (trong trường hợp chưa kích hoạt Turbo Cut): nâng dần từ 75 lên 85
    pub fn calculate_linear_epp(t_now: i32, target: i32, max_epp: u32) -> u32 {
        let dist = target - t_now;
        let epp = if dist >= 12 {
            30
        } else if dist <= 4 {
            if dist <= 1 {
                85
            } else {
                // dist = 4 -> 75, dist = 3 -> 78, dist = 2 -> 82
                75 + (4 - dist) as u32 * 3
            }
        } else {
            // 4 < dist < 12: nội suy tuyến tính từ 30 lên 75
            // dist = 12 -> 30, dist = 4 -> 75 (chênh lệch 45 trên 8 độ)
            let ratio = (12 - dist) as f32 / 8.0;
            (30.0 + ratio * 45.0).round() as u32
        };

        epp.clamp(0, max_epp)
    }

    /// Thuật toán thuần túy tính toán trạng thái điều phối kế tiếp `(PROCTHROTTLEMAX, PERFEPP)`.
    ///
    /// Cơ chế phối hợp 2 tầng:
    /// 1. Emergency Turbo Cut via PROCTHROTTLEMAX:
    ///    - Kích hoạt khi: `T_now >= target_temp - 1` HOẶC nhiệt độ dự báo `T_pred >= target_temp + 1`
    ///      với `T_pred = T_now + 2 * (T_now - T_prev)` (dự báo trước 400ms ở chu kỳ 200ms).
    ///    - Khi kích hoạt: Đặt `PROCTHROTTLEMAX = 99` (ngắt Turbo Boost) và `PERFEPP = 90`.
    ///    - Duy trì ngắt Turbo (Hysteresis): Giữ nguyên `(99, 90)` cho tới khi nhiệt độ hạ an toàn
    ///      `T_now <= target_temp - 4`.
    /// 2. Active Linear Modulation khi đã an toàn:
    ///    - Khôi phục `PROCTHROTTLEMAX = 100` (mở lại Turbo Boost).
    ///    - Điều biến `PERFEPP` tuyến tính từ 30 đến 75 tuỳ theo khoảng cách tới `target_temp`.
    pub fn calculate_next_state(
        current_throttle_max: u32,
        _current_epp: u32,
        current_cpu_temp: u32,
        prev_cpu_temp: Option<u32>,
        target_temp: u32,
        max_epp: u32,
    ) -> (u32, u32) {
        let t_now = current_cpu_temp as i32;
        let t_prev = prev_cpu_temp.map(|p| p as i32).unwrap_or(t_now);
        let delta = t_now - t_prev;
        let t_pred = t_now + 2 * delta;
        let target = target_temp as i32;

        let is_currently_cut = current_throttle_max < 100;

        // Điều kiện kích hoạt ngắt khẩn cấp Turbo Boost:
        // 1. Nhiệt độ hiện tại đã chạm hoặc vượt ngưỡng cảnh báo (target - 1)
        // 2. Tốc độ tăng nhiệt đột ngột dẫn tới dự báo vượt trần (T_pred > target, tức T_pred >= target + 1)
        let trigger_turbo_cut = t_now >= target - 1 || t_pred > target;

        if is_currently_cut {
            // Đang ở trạng thái Emergency Turbo Cut: Áp dụng Hysteresis 4°C
            // Chỉ mở lại Turbo khi T_now <= target - 4 và không có đà tăng nhiệt spike mới
            if t_now <= target - 4 && !trigger_turbo_cut {
                let next_epp = Self::calculate_linear_epp(t_now, target, max_epp);
                (100, next_epp)
            } else {
                // Tiếp tục duy trì ngắt Turbo để làm mát an toàn
                (99, 90.min(max_epp))
            }
        } else {
            // Đang ở trạng thái bình thường (PROCTHROTTLEMAX == 100)
            if trigger_turbo_cut {
                // Ngắt khẩn cấp Turbo Boost tức thì
                (99, 90.min(max_epp))
            } else {
                // Điều biến mượt EPP tuyến tính
                let next_epp = Self::calculate_linear_epp(t_now, target, max_epp);
                (100, next_epp)
            }
        }
    }

    /// Tương thích ngược: Tính toán mức EPP kế tiếp khi giữ PROCTHROTTLEMAX = 100.
    pub fn calculate_next_epp(
        current_epp: u32,
        current_cpu_temp: u32,
        prev_cpu_temp: Option<u32>,
        target_temp: u32,
        max_epp: u32,
    ) -> u32 {
        Self::calculate_next_state(
            100,
            current_epp,
            current_cpu_temp,
            prev_cpu_temp,
            target_temp,
            max_epp,
        ).1
    }

    /// Cập nhật vòng lặp điều khiển nhiệt độ với giá trị nhiệt độ CPU hiện tại (°C).
    /// Tự động áp dụng tối ưu hóa Win32 Power API (chỉ ghi khi thông số thay đổi).
    /// Trả về mức EPP (0 - max_epp) sau khi tính toán và áp dụng.
    pub fn update(&mut self, current_cpu_temp: u32) -> u32 {
        if !self.config.enabled {
            let safe_epp = if self.original_epp > 0 { self.original_epp } else { 50 };
            if self.current_epp != safe_epp || self.current_throttle_max != 100 {
                self.reset_to_maximum();
            }
            self.prev_cpu_temp = None;
            return self.current_epp;
        }

        let (next_throttle_max, next_epp) = Self::calculate_next_state(
            self.current_throttle_max,
            self.current_epp,
            current_cpu_temp,
            self.prev_cpu_temp,
            self.config.target_temp,
            self.config.max_epp,
        );

        if next_throttle_max != self.current_throttle_max || next_epp != self.current_epp {
            self.current_throttle_max = next_throttle_max;
            self.current_epp = next_epp;
            self.apply_power_settings(self.current_throttle_max, self.current_epp);
        }

        self.prev_cpu_temp = Some(current_cpu_temp);
        self.current_epp
    }

    /// Áp dụng đồng thời PROCTHROTTLEMAX và PERFEPP vào Windows Power Scheme qua `powrprof.dll`.
    ///
    /// Tối ưu hóa Win32 Power API:
    /// - Kiểm tra bộ nhớ cache `last_applied_throttle_max` và `last_applied_epp`.
    /// - Chỉ gọi `PowerWriteACValueIndex` / `PowerWriteDCValueIndex` cho tham số thực sự thay đổi.
    /// - Chỉ gọi `PowerSetActiveScheme` khi có ít nhất một giá trị thay đổi, tránh lãng phí chu kỳ CPU mỗi 200ms.
    pub fn apply_power_settings(&mut self, throttle_max: u32, epp_val: u32) -> bool {
        if !self.apply_to_hardware {
            self.last_applied_throttle_max = throttle_max;
            self.last_applied_epp = epp_val;
            return true;
        }

        if self.last_applied_throttle_max == throttle_max && self.last_applied_epp == epp_val {
            return true;
        }

        if let Some(ref scheme) = self.active_scheme_guid {
            unsafe {
                let mut changed = false;
                let mut all_success = true;

                if self.last_applied_throttle_max != throttle_max {
                    let write_max_ac = PowerWriteACValueIndex(
                        None,
                        scheme,
                        Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                        Some(&GUID_PROCESSOR_THROTTLE_MAXIMUM),
                        throttle_max,
                    );
                    let write_max_dc = PowerWriteDCValueIndex(
                        None,
                        scheme,
                        Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                        Some(&GUID_PROCESSOR_THROTTLE_MAXIMUM),
                        throttle_max,
                    );
                    if write_max_ac.0 == 0 && write_max_dc == 0 {
                        self.last_applied_throttle_max = throttle_max;
                        changed = true;
                    } else {
                        all_success = false;
                    }
                }

                if self.last_applied_epp != epp_val {
                    let write_epp_ac = PowerWriteACValueIndex(
                        None,
                        scheme,
                        Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                        Some(&GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE),
                        epp_val,
                    );
                    let write_epp_dc = PowerWriteDCValueIndex(
                        None,
                        scheme,
                        Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                        Some(&GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE),
                        epp_val,
                    );
                    if write_epp_ac.0 == 0 && write_epp_dc == 0 {
                        self.last_applied_epp = epp_val;
                        changed = true;
                    } else {
                        all_success = false;
                    }
                }

                if changed {
                    let apply_res = PowerSetActiveScheme(None, Some(scheme));
                    if apply_res.0 != 0 {
                        all_success = false;
                    }
                }

                all_success
            }
        } else {
            false
        }
    }

    /// Tương thích ngược: Áp dụng giá trị EPP với mức Throttle hiện tại.
    pub fn apply_epp(&mut self, epp_val: u32) -> bool {
        self.apply_power_settings(self.current_throttle_max, epp_val)
    }

    /// Khôi phục trạng thái vi xử lý về mức an toàn / gốc của hệ điều hành:
    /// - PROCTHROTTLEMAX = 100% (Mở lại Turbo Boost hoàn toàn)
    /// - PERFEPP = original_epp (nếu > 0) hoặc 50 (Windows Balanced mặc định)
    pub fn reset_to_maximum(&mut self) {
        self.current_throttle_max = 100;
        let restore_epp = if self.original_epp > 0 { self.original_epp } else { 50 };
        self.current_epp = restore_epp;
        self.prev_cpu_temp = None;
        self.apply_power_settings(100, restore_epp);
    }

    /// Khôi phục mức EPP và Throttle về giá trị ban đầu trước khi ứng dụng khởi chạy.
    pub fn reset_to_original(&mut self) {
        self.reset_to_maximum();
    }
}

impl Drop for ThermalGovernor {
    /// Khi ThermalGovernor bị giải phóng bộ nhớ (Drop), tự động khôi phục PROCTHROTTLEMAX = 100%
    /// và PERFEPP = original_epp (hoặc 50) để đảm bảo CPU không bị kẹt ở trạng thái giảm hiệu năng.
    fn drop(&mut self) {
        self.reset_to_maximum();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emergency_turbo_cut_when_near_target() {
        let target = 75;
        let max_epp = 90;

        // 1. T_now = 74 (chạm mốc target - 1): Kích hoạt ngay ngắt Turbo và đặt EPP = 90
        let (throttle, epp) = ThermalGovernor::calculate_next_state(100, 30, 74, None, target, max_epp);
        assert_eq!(throttle, 99, "T_now = target - 1 (74°C) phải ngắt Turbo (PROCTHROTTLEMAX = 99)");
        assert_eq!(epp, 90, "T_now = target - 1 phải đặt EPP = 90");

        // 2. T_now = 75 (bằng target): Ngắt Turbo và EPP = 90
        let (throttle_75, epp_75) = ThermalGovernor::calculate_next_state(100, 30, 75, None, target, max_epp);
        assert_eq!(throttle_75, 99);
        assert_eq!(epp_75, 90);

        // 3. T_now = 80 (vượt target): Ngắt Turbo và EPP = 90
        let (throttle_80, epp_80) = ThermalGovernor::calculate_next_state(100, 30, 80, None, target, max_epp);
        assert_eq!(throttle_80, 99);
        assert_eq!(epp_80, 90);
    }

    #[test]
    fn test_predictive_turbo_cut_on_thermal_spike() {
        let target = 75;
        let max_epp = 90;

        // Kịch bản: T_now = 72°C (chưa chạm target - 1 = 74°C), nhưng tốc độ tăng nhiệt rất dốc:
        // T_prev = 68°C -> delta = +4°C -> T_pred = 72 + 2 * 4 = 80°C >= target + 1 (76°C)
        let (throttle, epp) = ThermalGovernor::calculate_next_state(100, 30, 72, Some(68), target, max_epp);
        assert_eq!(throttle, 99, "Spike dốc (T_pred = 80 >= 76) phải đón đầu ngắt ngay Turbo Boost");
        assert_eq!(epp, 90, "Spike dốc phải ép EPP lên 90");

        // Kịch bản: T_now = 71°C, T_prev = 70°C (delta = +1°C) -> T_pred = 71 + 2 = 73 < 76°C
        // Không kích hoạt Turbo Cut, điều biến tuyến tính
        let (throttle_normal, epp_normal) = ThermalGovernor::calculate_next_state(100, 30, 71, Some(70), target, max_epp);
        assert_eq!(throttle_normal, 100, "Tăng chậm dưới ngưỡng không được ngắt Turbo");
        assert_eq!(epp_normal, 75, "T_now = 71 (target - 4) có EPP = 75");
    }

    #[test]
    fn test_hysteresis_turbo_cut_retention_and_recovery() {
        let target = 75;
        let max_epp = 90;

        // Đang trong trạng thái Turbo Cut (throttle = 99, epp = 90)
        // 1. Làm mát về 73°C (> target - 4 = 71°C): Vẫn phải giữ nguyên Turbo Cut
        let (throttle_73, epp_73) = ThermalGovernor::calculate_next_state(99, 90, 73, Some(75), target, max_epp);
        assert_eq!(throttle_73, 99, "73°C > 71°C chưa đủ điều kiện thoát Hysteresis, phải giữ Turbo Cut");
        assert_eq!(epp_73, 90);

        // 2. Làm mát về 72°C (> 71°C): Vẫn giữ nguyên Turbo Cut
        let (throttle_72, epp_72) = ThermalGovernor::calculate_next_state(99, 90, 72, Some(73), target, max_epp);
        assert_eq!(throttle_72, 99);
        assert_eq!(epp_72, 90);

        // 3. Làm mát an toàn về 71°C (<= target - 4): Thoát Turbo Cut, khôi phục 100% Throttle và EPP = 75
        let (throttle_71, epp_71) = ThermalGovernor::calculate_next_state(99, 90, 71, Some(72), target, max_epp);
        assert_eq!(throttle_71, 100, "71°C (target - 4) đã hạ an toàn, khôi phục PROCTHROTTLEMAX = 100");
        assert_eq!(epp_71, 75, "Tại target - 4, EPP điều biến về 75");

        // 4. Tiếp tục làm mát về 67°C (target - 8): Throttle = 100, EPP = 53
        let (throttle_67, epp_67) = ThermalGovernor::calculate_next_state(100, 75, 67, Some(71), target, max_epp);
        assert_eq!(throttle_67, 100);
        assert_eq!(epp_67, 53);
    }

    #[test]
    fn test_linear_epp_modulation_safe_zone() {
        let target = 75;
        let max_epp = 90;

        // Cách xa mục tiêu >= 12°C: EPP = 30
        assert_eq!(ThermalGovernor::calculate_linear_epp(63, target, max_epp), 30, "target - 12 (63°C) -> EPP = 30");
        assert_eq!(ThermalGovernor::calculate_linear_epp(50, target, max_epp), 30, "Mát sâu (50°C) -> EPP = 30");
        assert_eq!(ThermalGovernor::calculate_linear_epp(35, target, max_epp), 30, "Idle mát lạnh (35°C) -> EPP = 30");

        // Dải tuyến tính giữa target - 12 (63°C) và target - 4 (71°C)
        assert_eq!(ThermalGovernor::calculate_linear_epp(65, target, max_epp), 41, "dist = 10 -> EPP = 41");
        assert_eq!(ThermalGovernor::calculate_linear_epp(67, target, max_epp), 53, "dist = 8 -> EPP = 53");
        assert_eq!(ThermalGovernor::calculate_linear_epp(69, target, max_epp), 64, "dist = 6 -> EPP = 64");
        assert_eq!(ThermalGovernor::calculate_linear_epp(71, target, max_epp), 75, "dist = 4 -> EPP = 75");

        // Khi tiệm cận mục tiêu (dist = 3, 2)
        assert_eq!(ThermalGovernor::calculate_linear_epp(72, target, max_epp), 78, "dist = 3 -> EPP = 78");
        assert_eq!(ThermalGovernor::calculate_linear_epp(73, target, max_epp), 81, "dist = 2 -> EPP = 81");
    }

    #[test]
    fn test_governor_update_stateful_lifecycle() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 75,
            tolerance: 0,
            max_epp: 90,
        };

        let mut governor = ThermalGovernor::new_test(config);
        assert_eq!(governor.current_throttle_max, 100);
        assert_eq!(governor.current_epp, 0);

        // 1. T_now = 60°C: Cách xa mục tiêu (> 12°C) -> Throttle = 100, EPP = 30
        assert_eq!(governor.update(60), 30);
        assert_eq!(governor.current_throttle_max, 100);
        assert!(!governor.is_turbo_cut());

        // 2. Tăng nhiệt từ từ: 65°C rồi lên 67°C (delta = +2 -> T_pred = 67 + 4 = 71 < 76)
        governor.update(65);
        assert_eq!(governor.update(67), 53);
        assert_eq!(governor.current_throttle_max, 100);

        // 3. Tăng tiếp từ từ: 69°C rồi lên 71°C (delta = +2 -> T_pred = 71 + 4 = 75 < 76)
        governor.update(69);
        assert_eq!(governor.update(71), 75);
        assert_eq!(governor.current_throttle_max, 100);

        // 4. Spike nhảy từ 71°C lên 74°C (delta = +3 -> T_pred = 74 + 6 = 80 >= 76)
        // và 74 == target - 1 -> Ngắt Turbo tức thì!
        assert_eq!(governor.update(74), 90);
        assert_eq!(governor.current_throttle_max, 99);
        assert!(governor.is_turbo_cut());

        // 5. Làm mát dần: 73°C -> Vẫn giữ Turbo Cut do Hysteresis
        assert_eq!(governor.update(73), 90);
        assert_eq!(governor.current_throttle_max, 99);

        // 6. Làm mát tiếp: 72°C -> Vẫn giữ Turbo Cut
        assert_eq!(governor.update(72), 90);
        assert_eq!(governor.current_throttle_max, 99);

        // 7. Làm mát an toàn: 71°C (target - 4) -> Mở lại Turbo Boost và EPP = 75
        assert_eq!(governor.update(71), 75);
        assert_eq!(governor.current_throttle_max, 100);
        assert!(!governor.is_turbo_cut());

        // 8. Tiếp tục mát: 63°C -> EPP = 30
        assert_eq!(governor.update(63), 30);
        assert_eq!(governor.current_throttle_max, 100);
    }

    #[test]
    fn test_power_api_caching_and_avoiding_spam() {
        let config = ThermalGovernorConfig::default();
        let mut governor = ThermalGovernor::new_test(config);

        governor.last_applied_throttle_max = 100;
        governor.last_applied_epp = 50;

        // Gọi apply với cùng giá trị: không làm gì, trả về true
        assert!(governor.apply_power_settings(100, 50));
        assert_eq!(governor.last_applied_throttle_max, 100);
        assert_eq!(governor.last_applied_epp, 50);

        // Thay đổi giá trị: cập nhật cache
        assert!(governor.apply_power_settings(99, 90));
        assert_eq!(governor.last_applied_throttle_max, 99);
        assert_eq!(governor.last_applied_epp, 90);
    }

    #[test]
    fn test_fail_safe_cleanup_and_drop() {
        let mut governor = ThermalGovernor::new_test(ThermalGovernorConfig::default());
        governor.original_epp = 50;
        governor.current_throttle_max = 99;
        governor.current_epp = 90;
        governor.prev_cpu_temp = Some(75);

        // Gọi reset_to_maximum: phải khôi phục 100% Throttle và EPP = original_epp (50)
        governor.reset_to_maximum();
        assert_eq!(governor.current_throttle_max, 100);
        assert_eq!(governor.current_epp, 50);
        assert_eq!(governor.prev_cpu_temp, None);
        assert_eq!(governor.last_applied_throttle_max, 100);
        assert_eq!(governor.last_applied_epp, 50);

        // Kiểm tra khi governor bị disabled trong update()
        governor.config.enabled = false;
        governor.current_throttle_max = 99;
        governor.current_epp = 90;
        let res = governor.update(85);
        assert_eq!(res, 50);
        assert_eq!(governor.current_throttle_max, 100);
        assert_eq!(governor.current_epp, 50);
    }

    #[test]
    fn test_config_defaults() {
        let cfg = ThermalGovernorConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.target_temp, 80);
        assert_eq!(cfg.tolerance, 0);
        assert_eq!(cfg.max_epp, 90);
    }
}
