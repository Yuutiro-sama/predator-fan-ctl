//! Module Vi điều khiển Năng lượng & Nhiệt độ Động (Closed-Loop Thermal Governor).
//!
//! Quản lý và điều phối mượt mà trạng thái năng lượng vi xử lý thông qua
//! Intel Speed Shift Energy Performance Preference (`PERFEPP`), giúp duy trì nhiệt độ
//! mục tiêu 80°C ± 3°C một cách êm ái, loại bỏ hoàn toàn hiện tượng khựng khung hình (Zero Stuttering),
//! đồng thời TUYỆT ĐỐI KHÔNG BAO GIỜ tắt Intel Turbo Boost (PROCTHROTTLEMAX luôn giữ 100%).

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
/// Lưu ý: Luôn luôn được ghim ở mức 100% để Windows không tắt Intel Turbo Boost!
pub const GUID_PROCESSOR_THROTTLE_MAXIMUM: GUID = GUID {
    data1: 0xbc5038f7,
    data2: 0x23e0,
    data3: 0x4960,
    data4: [0x96, 0xda, 0x33, 0xab, 0xaf, 0x59, 0x35, 0xec],
};

/// GUID thiết lập thiên hướng hiệu năng / năng lượng (Energy Performance Preference / PERFEPP)
/// Value: 36687f9e-e3a5-4dbf-b1dc-15eb381c6863
/// Dải giá trị:
///   0: Maximum Performance (Intel Speed Shift duy trì 4.5 - 4.9 GHz Boost liên tục)
///  25: High Performance Smooth (~4.0 - 4.5 GHz)
///  35: Balanced Gaming (~3.8 - 4.2 GHz)
///  50: Balanced (~3.4 - 3.8 GHz)
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
    3
}

pub const fn default_max_epp() -> u32 {
    50
}

/// Cấu hình tham số điều khiển bộ điều phối nhiệt độ CPU (Thermal Governor Configuration).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThermalGovernorConfig {
    /// Bật / tắt bộ điều phối nhiệt độ động
    #[serde(default = "default_governor_enabled")]
    pub enabled: bool,

    /// Nhiệt độ mục tiêu (°C) - mặc định 80°C
    #[serde(default = "default_target_temp", alias = "target_cpu_temp")]
    pub target_temp: u32,

    /// Dải dung sai ổn định nhiệt độ (±°C) - mặc định 3°C (dải 77°C - 83°C)
    #[serde(default = "default_tolerance")]
    pub tolerance: u32,

    /// Giới hạn EPP tối đa (sàn an toàn, không tăng EPP quá giá trị này để tránh giật lag máy) - mặc định 50
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

/// Bộ điều khiển nhiệt độ CPU động vòng kín qua Intel Speed Shift EPP (Smooth EPP Thermal Governor).
/// Tương tác trực tiếp với Win32 Power Management subsystem để ghim nhiệt độ tại 80°C ± 3°C
/// mà không bao giờ tắt Intel Turbo Boost.
pub struct ThermalGovernor {
    /// Cấu hình hoạt động
    pub config: ThermalGovernorConfig,

    /// Mức EPP hiện tại đang áp dụng (0 = Maximum Boost, 50 = Balanced)
    pub current_epp: u32,

    /// Mức EPP gốc đọc từ hệ điều hành lúc khởi động để khôi phục
    pub original_epp: u32,

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

        let governor = Self {
            config,
            current_epp: 0,
            original_epp,
            active_scheme_guid,
            apply_to_hardware: true,
        };

        governor.apply_epp(0);
        governor
    }

    /// Khởi tạo phiên bản kiểm thử không can thiệp trực tiếp vào thiết lập nguồn của hệ điều hành.
    pub fn new_test(config: ThermalGovernorConfig) -> Self {
        let mut governor = Self::new(config);
        governor.apply_to_hardware = false;
        governor
    }

    /// Hàm nội bộ đọc Active Power Scheme GUID, ghi đè PROCTHROTTLEMAX = 100% và đọc EPP gốc từ Windows
    fn init_power_scheme() -> (Option<GUID>, u32) {
        unsafe {
            let mut p_guid: *mut GUID = std::ptr::null_mut();
            let res = PowerGetActiveScheme(None, &mut p_guid);
            if res.0 != 0 || p_guid.is_null() {
                return (None, 0);
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
            let orig = if read_res.0 == 0 { orig_val } else { 0 };

            // TUYỆT ĐỐI KHÔNG HẠ PROCTHROTTLEMAX: Luôn ghim PROCTHROTTLEMAX = 100% để giữ Turbo Boost
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

    /// Thuật toán thuần túy tính toán mức EPP kế tiếp dựa trên nhiệt độ CPU đo được (Pure Function).
    ///
    /// Quy tắc điều tiết:
    /// 1. Vượt ngưỡng dung sai `current_cpu_temp > target_temp + tolerance` (> 83°C):
    ///    - Nếu vọt quá cao `current_cpu_temp >= target_temp + tolerance * 2` (>= 86°C): Tăng mạnh EPP +10.
    ///    - Nếu chớm qua ngưỡng (84°C - 85°C): Tăng EPP +5.
    /// 2. Vượt mục tiêu nhưng trong dung sai `current_cpu_temp > target_temp` (81°C - 83°C):
    ///    - Tăng nhẹ EPP +2 để hãm đà tăng nhiệt êm dịu, không giật lag.
    /// 3. Dải ổn định `target_temp - tolerance <= current_cpu_temp <= target_temp` (77°C - 80°C):
    ///    - Deadband: Giữ nguyên EPP để triệt tiêu dao động xung nhịp (anti-oscillation).
    /// 4. Dưới dải ổn định `current_cpu_temp < target_temp - tolerance` (< 77°C):
    ///    - Nếu rất mát `current_cpu_temp <= target_temp - tolerance * 2` (<= 74°C): Giảm nhanh EPP -5 về 0.
    ///    - Nếu vừa mát (75°C - 76°C): Giảm dần EPP -3 về 0.
    /// 5. Kẹp giá trị vào khoảng `[0, max_epp]`.
    pub fn calculate_next_epp(
        current_epp: u32,
        current_cpu_temp: u32,
        target_temp: u32,
        tolerance: u32,
        max_epp: u32,
    ) -> u32 {
        let lower_bound = target_temp.saturating_sub(tolerance);
        let upper_bound = target_temp.saturating_add(tolerance);
        let upper_spike = target_temp.saturating_add(tolerance * 2);
        let lower_spike = target_temp.saturating_sub(tolerance * 2);

        let next = if current_cpu_temp > upper_bound {
            let step = if current_cpu_temp >= upper_spike {
                10
            } else {
                5
            };
            current_epp.saturating_add(step)
        } else if current_cpu_temp > target_temp {
            current_epp.saturating_add(2)
        } else if current_cpu_temp >= lower_bound {
            // Deadband / Steady state: giữ nguyên EPP để chống hiện tượng săn xung
            current_epp
        } else {
            let step = if current_cpu_temp <= lower_spike {
                5
            } else {
                3
            };
            current_epp.saturating_sub(step)
        };

        next.clamp(0, max_epp)
    }

    /// Cập nhật vòng lặp điều khiển nhiệt độ với giá trị nhiệt độ CPU hiện tại (°C).
    /// Trả về mức EPP (0 - max_epp) sau khi tính toán và áp dụng.
    pub fn update(&mut self, current_cpu_temp: u32) -> u32 {
        if !self.config.enabled {
            if self.current_epp != 0 {
                self.reset_to_maximum();
            }
            return self.current_epp;
        }

        let next_epp = Self::calculate_next_epp(
            self.current_epp,
            current_cpu_temp,
            self.config.target_temp,
            self.config.tolerance,
            self.config.max_epp,
        );

        if next_epp != self.current_epp {
            self.current_epp = next_epp;
            self.apply_epp(self.current_epp);
        }

        self.current_epp
    }

    /// Áp dụng giá trị EPP và đảm bảo PROCTHROTTLEMAX = 100% vào Windows Power Scheme qua `powrprof.dll`.
    pub fn apply_epp(&self, epp_val: u32) -> bool {
        if !self.apply_to_hardware {
            return true;
        }

        if let Some(ref scheme) = self.active_scheme_guid {
            unsafe {
                // 1. Luôn ghim PROCTHROTTLEMAX = 100% cho cả AC và DC để đảm bảo không bao giờ tắt Intel Turbo Boost
                let write_max_ac = PowerWriteACValueIndex(
                    None,
                    scheme,
                    Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                    Some(&GUID_PROCESSOR_THROTTLE_MAXIMUM),
                    100,
                );
                let write_max_dc = PowerWriteDCValueIndex(
                    None,
                    scheme,
                    Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                    Some(&GUID_PROCESSOR_THROTTLE_MAXIMUM),
                    100,
                );

                // 2. Ghi giá trị EPP mới vào Windows Power Scheme cho cả AC và DC
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

                // 3. Kích hoạt scheme ngay lập tức để Windows áp dụng
                let apply_res = PowerSetActiveScheme(None, Some(scheme));

                write_max_ac.0 == 0
                    && write_max_dc == 0
                    && write_epp_ac.0 == 0
                    && write_epp_dc == 0
                    && apply_res.0 == 0
            }
        } else {
            false
        }
    }

    /// Khôi phục CPU về hiệu năng tối đa (EPP = 0, PROCTHROTTLEMAX = 100%).
    /// Đảm bảo CPU hoạt động ở mức Turbo Boost cao nhất sau khi đóng ứng dụng hoặc tắt governor.
    pub fn reset_to_maximum(&mut self) {
        self.current_epp = 0;
        self.apply_epp(0);
    }

    /// Khôi phục mức EPP về giá trị ban đầu trước khi ứng dụng khởi chạy.
    pub fn reset_to_original(&mut self) {
        self.current_epp = self.original_epp;
        self.apply_epp(self.original_epp);
    }
}

impl Drop for ThermalGovernor {
    /// Khi ThermalGovernor bị giải phóng bộ nhớ (Drop), tự động khôi phục EPP = 0 và PROCTHROTTLEMAX = 100%.
    fn drop(&mut self) {
        self.reset_to_maximum();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_epp_throttling_above_target() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 80,
            tolerance: 3,
            max_epp: 50,
        };

        let mut governor = ThermalGovernor::new_test(config);
        assert_eq!(governor.current_epp, 0, "Khởi đầu EPP phải là 0 (Max Performance)");

        // 1. Vượt nhẹ mục tiêu nhưng trong dung sai (81°C - 83°C): Tăng nhẹ EPP +2
        let res_82 = governor.update(82);
        assert_eq!(res_82, 2, "82°C (chớm quá target) phải tăng EPP +2 từ 0 lên 2");

        // 2. Vượt dung sai (84°C - 85°C): Tăng EPP +5
        let res_85 = governor.update(85);
        assert_eq!(res_85, 7, "85°C (> 83°C) phải tăng EPP +5 từ 2 lên 7");

        // 3. Vượt rất cao (>= 86°C): Tăng khẩn cấp EPP +10
        let res_90 = governor.update(90);
        assert_eq!(res_90, 17, "90°C (>= 86°C) phải tăng EPP +10 từ 7 lên 17");

        let res_92 = governor.update(92);
        assert_eq!(res_92, 27, "92°C phải tiếp tục tăng EPP +10 từ 17 lên 27");
    }

    #[test]
    fn test_step_epp_unthrottling_below_target() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 80,
            tolerance: 3,
            max_epp: 50,
        };

        let mut governor = ThermalGovernor::new_test(config);
        governor.current_epp = 30; // Giả lập trạng thái đang chạy ở mức EPP = 30

        // 1. Mát dưới ngưỡng dung sai nhưng chưa sâu (75°C - 76°C): Giảm dần EPP -3
        let res_75 = governor.update(75);
        assert_eq!(res_75, 27, "75°C (< 77°C) phải giảm EPP -3 từ 30 xuống 27");

        // 2. Rất mát (<= 74°C): Giảm nhanh EPP -5
        let res_65 = governor.update(65);
        assert_eq!(res_65, 22, "65°C (<= 74°C) phải giảm nhanh EPP -5 từ 27 xuống 22");

        let res_60 = governor.update(60);
        assert_eq!(res_60, 17, "60°C phải tiếp tục giảm nhanh EPP -5 xuống 17");

        // Giảm dần nhiều bước đến khi chạm ngưỡng sàn EPP = 0 (Maximum Performance)
        for _ in 0..10 {
            governor.update(50);
        }
        assert_eq!(governor.current_epp, 0, "Phải hồi phục hoàn toàn về EPP = 0");
    }

    #[test]
    fn test_deadband_stability() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 80,
            tolerance: 3, // Dải deadband: 77°C - 80°C
            max_epp: 50,
        };

        let mut governor = ThermalGovernor::new_test(config);
        governor.current_epp = 25; // Đang ổn định ở mức EPP 25

        // Thử nghiệm toàn bộ các mốc nhiệt độ nằm trong vùng Deadband [77, 80]
        assert_eq!(governor.update(77), 25, "77°C thuộc deadband, không đổi EPP");
        assert_eq!(governor.update(78), 25, "78°C thuộc deadband, không đổi EPP");
        assert_eq!(governor.update(79), 25, "79°C thuộc deadband, không đổi EPP");
        assert_eq!(governor.update(80), 25, "80°C thuộc deadband, không đổi EPP");
        assert_eq!(governor.current_epp, 25, "Trạng thái EPP phải được giữ nguyên tuyệt đối");
    }

    #[test]
    fn test_epp_clamping() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 80,
            tolerance: 3,
            max_epp: 50,
        };

        let mut governor = ThermalGovernor::new_test(config);

        // Kiểm tra kẹp trần (Max Clamping với max_epp = 50):
        governor.current_epp = 45;
        let res = governor.update(95); // Nhiệt độ 95°C đòi hỏi tăng +10, 45 + 10 = 55 > 50
        assert_eq!(res, 50, "EPP không được vượt quá max_epp = 50");
        assert_eq!(governor.current_epp, 50);

        // Tiếp tục quá nhiệt khi đã ở trần 50
        let res_ceiling = governor.update(100);
        assert_eq!(res_ceiling, 50, "Khi đã ở trần 50, không bao giờ tăng thêm");

        // Kiểm tra kẹp sàn (Min Clamping về 0):
        governor.current_epp = 2;
        let res_min = governor.update(50); // Nhiệt độ 50°C đòi hỏi giảm -5, 2 saturating_sub 5 = 0
        assert_eq!(res_min, 0, "EPP không được giảm dưới 0");
        assert_eq!(governor.current_epp, 0);

        let res_floor = governor.update(45);
        assert_eq!(res_floor, 0, "Khi đã ở sàn 0, không bao giờ giảm thêm");
    }

    #[test]
    fn test_governor_disabled_reset() {
        let mut config = ThermalGovernorConfig::default();
        config.enabled = false;

        let mut governor = ThermalGovernor::new_test(config);
        governor.current_epp = 35;

        // Khi governor bị tắt, update phải tự khôi phục về EPP = 0 (Max Performance)
        let res = governor.update(95);
        assert_eq!(res, 0);
        assert_eq!(governor.current_epp, 0);
    }

    #[test]
    fn test_reset_to_maximum() {
        let mut governor = ThermalGovernor::new_test(ThermalGovernorConfig::default());
        governor.current_epp = 40;
        governor.reset_to_maximum();
        assert_eq!(governor.current_epp, 0);
    }

    #[test]
    fn test_config_defaults() {
        let cfg = ThermalGovernorConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.target_temp, 80);
        assert_eq!(cfg.tolerance, 3);
        assert_eq!(cfg.max_epp, 50);
    }
}
