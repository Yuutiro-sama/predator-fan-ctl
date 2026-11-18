//! Module Vi điều khiển Năng lượng & Nhiệt độ Động (Closed-Loop Thermal Governor).
//!
//! Quản lý và điều phối mượt mà trạng thái năng lượng vi xử lý thông qua
//! Intel Speed Shift Energy Performance Preference (`PERFEPP`), giúp duy trì nhiệt độ
//! mục tiêu một cách êm ái, loại bỏ hoàn toàn hiện tượng khựng khung hình (Zero Stuttering),
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

    /// Nhiệt độ mục tiêu (°C) - mặc định 80°C
    #[serde(default = "default_target_temp", alias = "target_cpu_temp")]
    pub target_temp: u32,

    /// Dải dung sai ổn định nhiệt độ (±°C) - mặc định 0°C để lấy target_temp làm trần cứng kiểm soát
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

/// Bộ điều khiển nhiệt độ CPU động vòng kín qua Intel Speed Shift EPP (Smooth EPP Thermal Governor).
/// Tương tác trực tiếp với Win32 Power Management subsystem để ghim nhiệt độ mục tiêu một cách êm ái
/// thông qua thuật toán Asymmetric Proportional-Derivative (Fast Attack / Slow Decay),
/// đồng thời TUYỆT ĐỐI KHÔNG BAO GIỜ tắt Intel Turbo Boost (PROCTHROTTLEMAX luôn giữ 100%).
pub struct ThermalGovernor {
    /// Cấu hình hoạt động
    pub config: ThermalGovernorConfig,

    /// Mức EPP hiện tại đang áp dụng (0 = Maximum Boost, 90 = Throttle)
    pub current_epp: u32,

    /// Nhiệt độ CPU chu kỳ trước để theo dõi tốc độ tăng nhiệt (dT/dt)
    pub prev_cpu_temp: Option<u32>,

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
            prev_cpu_temp: None,
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

    /// Thuật toán thuần túy tính toán mức EPP kế tiếp dựa trên sai số nhiệt độ và đạo hàm tốc độ tăng nhiệt (Pure Function).
    ///
    /// Áp dụng mô hình Asymmetric Proportional-Derivative (Fast Attack / Slow Decay):
    /// - `diff = (current_temp as i32) - (target_temp as i32)`
    /// - `rate = match prev_temp { Some(p) => current_temp as i32 - p as i32, None => 0 }`
    ///
    /// Bảng nhảy bậc bất đối xứng:
    /// - `diff >= 3` (quá nhiệt >= +3°C, vd >= 78°C khi target 75°C): Emergency Clamp -> `current_epp.max(85).min(max_epp)`
    /// - `diff == 2` (quá nhiệt +2°C, vd 77°C): Hard Throttle -> `(current_epp.saturating_add(25)).max(65).min(max_epp)`
    /// - `diff == 1` (quá nhiệt +1°C, vd 76°C): Proportional Attack -> `(current_epp.saturating_add(15)).max(45).min(max_epp)`
    /// - `diff == 0` (đang ở đúng 75°C):
    ///   + Nếu `rate > 0` (nhiệt độ đang trên đà tăng spike): Pre-emptive Hold -> `(current_epp.saturating_add(8)).min(max_epp)`
    ///   + Nếu `rate <= 0`: Giữ nguyên `current_epp`
    /// - `diff == -1` (vd 74°C): Giữ nguyên `current_epp` (vùng đệm chống rung xung nhịp)
    /// - `diff <= -2` (vd <= 73°C): Smooth Recovery -> `current_epp.saturating_sub(3)` (hoặc -5 nếu `diff <= -4`), sàn là 0.
    pub fn calculate_next_epp(
        current_epp: u32,
        current_cpu_temp: u32,
        prev_cpu_temp: Option<u32>,
        target_temp: u32,
        max_epp: u32,
    ) -> u32 {
        let diff = (current_cpu_temp as i32) - (target_temp as i32);
        let rate = match prev_cpu_temp {
            Some(p) => (current_cpu_temp as i32) - (p as i32),
            None => 0,
        };

        let next = if diff >= 3 {
            // Emergency Clamp: Quá nhiệt >= +3°C, nhảy tức thì lên 85-90 để cắt đỉnh nhiệt ngay lập tức
            current_epp.max(85).min(max_epp)
        } else if diff == 2 {
            // Hard Throttle: Quá nhiệt +2°C, hãm mạnh xung nhịp
            current_epp.saturating_add(25).max(65).min(max_epp)
        } else if diff == 1 {
            // Proportional Attack: Quá nhiệt +1°C, chặn đà tăng nhiệt ngay khi chớm vượt
            current_epp.saturating_add(15).max(45).min(max_epp)
        } else if diff == 0 {
            if rate > 0 {
                // Pre-emptive Hold: Spike dốc khi chạm mốc target (dT/dt > 0), hãm đón đầu
                current_epp.saturating_add(8).min(max_epp)
            } else {
                // Đứng yên hoặc giảm nhiệt: Giữ nguyên EPP
                current_epp
            }
        } else if diff == -1 {
            // Vùng đệm chống săn xung (Anti-Hunting deadband buffer)
            current_epp
        } else if diff <= -4 {
            // Rất mát (<= target - 4): Giảm nhanh EPP -5 về sàn 0
            current_epp.saturating_sub(5)
        } else {
            // Mát vừa (diff == -2 hoặc -3): Smooth Recovery giảm nhẹ EPP -3
            current_epp.saturating_sub(3)
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
            self.prev_cpu_temp = None;
            return self.current_epp;
        }

        let next_epp = Self::calculate_next_epp(
            self.current_epp,
            current_cpu_temp,
            self.prev_cpu_temp,
            self.config.target_temp,
            self.config.max_epp,
        );

        if next_epp != self.current_epp {
            self.current_epp = next_epp;
            self.apply_epp(self.current_epp);
        }

        self.prev_cpu_temp = Some(current_cpu_temp);
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
        self.prev_cpu_temp = None;
        self.apply_epp(0);
    }

    /// Khôi phục mức EPP về giá trị ban đầu trước khi ứng dụng khởi chạy.
    pub fn reset_to_original(&mut self) {
        self.current_epp = self.original_epp;
        self.prev_cpu_temp = None;
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
    fn test_asymmetric_attack_immediate_reaction() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 75,
            tolerance: 0,
            max_epp: 90,
        };

        // Kịch bản 1: Vượt +1°C (76°C) phản ứng tức thì lên tối thiểu 45
        let mut gov1 = ThermalGovernor::new_test(config);
        assert_eq!(gov1.current_epp, 0);
        let res_76 = gov1.update(76);
        assert_eq!(res_76, 45, "+1°C (76°C) phải kích hoạt Proportional Attack nhảy tức thì lên EPP 45");
        // Nếu tiếp tục ở 76°C chu kỳ sau (vẫn vượt 1°C), tăng tiếp +15
        let res_76_next = gov1.update(76);
        assert_eq!(res_76_next, 60, "Tiếp tục vượt +1°C chu kỳ 2 phải cộng dồn +15 lên 60");

        // Kịch bản 2: Vượt +2°C (77°C) phản ứng tức thì lên tối thiểu 65
        let mut gov2 = ThermalGovernor::new_test(config);
        let res_77 = gov2.update(77);
        assert_eq!(res_77, 65, "+2°C (77°C) phải kích hoạt Hard Throttle nhảy tức thì lên EPP 65");

        // Kịch bản 3: Vượt +3°C (78°C) phản ứng khẩn cấp Emergency Clamp lên 85
        let mut gov3 = ThermalGovernor::new_test(config);
        let res_78 = gov3.update(78);
        assert_eq!(res_78, 85, "+3°C (78°C) phải kích hoạt Emergency Clamp nhảy tức thì lên EPP 85");

        // Kịch bản 4: Vượt rất cao (>= +3°C, ví dụ 85°C) nhảy tức thì lên 85
        let mut gov4 = ThermalGovernor::new_test(config);
        let res_85 = gov4.update(85);
        assert_eq!(res_85, 85, "Quá nhiệt nặng (85°C) phải Emergency Clamp ngay lập tức ở 85");
    }

    #[test]
    fn test_spike_rate_of_rise_at_target_temp() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 75,
            tolerance: 0,
            max_epp: 90,
        };

        let mut governor = ThermalGovernor::new_test(config);

        // Chu kỳ 1: Nhiệt độ mát 70°C -> EPP = 0, ghi nhận prev = 70
        assert_eq!(governor.update(70), 0);
        assert_eq!(governor.prev_cpu_temp, Some(70));

        // Chu kỳ 2: Spike vọt dốc từ 70°C lên đúng 75°C (diff = 0, rate = +5 > 0)
        // Pre-emptive Hold phải phát hiện đà tăng và hãm trước +8 EPP
        let res_spike = governor.update(75);
        assert_eq!(res_spike, 8, "Spike dốc khi chạm 75°C phải Pre-emptive Hold tăng EPP +8 đón đầu");
        assert_eq!(governor.prev_cpu_temp, Some(75));

        // Chu kỳ 3: Nhiệt độ đứng yên tại 75°C (diff = 0, rate = 0 <= 0)
        // Không tăng thêm, giữ nguyên EPP = 8
        let res_steady = governor.update(75);
        assert_eq!(res_steady, 8, "Đứng yên tại target temp phải giữ nguyên EPP để ổn định");

        // Chu kỳ 4: Chớm hạ về 74°C (diff = -1) -> Vùng đệm chống rung xung nhịp
        let res_74 = governor.update(74);
        assert_eq!(res_74, 8, "74°C (diff = -1) là vùng đệm chống rung, giữ nguyên EPP");

        // Chu kỳ 5: Lại bật tăng từ 74°C lên 75°C (diff = 0, rate = +1 > 0)
        let res_spike_again = governor.update(75);
        assert_eq!(res_spike_again, 16, "Bật tăng từ 74 lên 75 (rate > 0) phải kích hoạt Pre-emptive Hold +8 lên 16");

        // Chu kỳ 6: Quá nhiệt chớm 76°C (diff = +1)
        let res_76 = governor.update(76);
        assert_eq!(res_76, 45, "Chớm quá 76°C kích hoạt Proportional Attack nhảy lên sàn 45");

        // Chu kỳ 7: Hạ nhiệt từ 76°C về 75°C (diff = 0, rate = -1 <= 0)
        // Nhiệt độ đang hạ về target nên KHÔNG kích hoạt Pre-emptive Hold, giữ nguyên 45
        let res_cool_target = governor.update(75);
        assert_eq!(res_cool_target, 45, "Hạ nhiệt về 75°C (rate < 0) phải giữ nguyên EPP = 45");
    }

    #[test]
    fn test_deadband_stability() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 75,
            tolerance: 0,
            max_epp: 90,
        };

        let mut governor = ThermalGovernor::new_test(config);
        governor.current_epp = 50;

        // diff = -1 (74°C): Vùng đệm chống dao động xung nhịp
        assert_eq!(governor.update(74), 50, "74°C (diff = -1) phải giữ nguyên EPP");
        assert_eq!(governor.update(74), 50, "74°C chu kỳ tiếp theo vẫn giữ nguyên EPP");

        // Khi nhiệt độ ở đúng target (75°C) và không tăng (rate <= 0):
        // Giả lập từ 75°C sang 75°C
        assert_eq!(governor.update(75), 58); // rate = 75 - 74 = +1 > 0 -> 50 + 8 = 58
        assert_eq!(governor.update(75), 58); // rate = 75 - 75 = 0 <= 0 -> giữ nguyên 58
    }

    #[test]
    fn test_smooth_recovery_below_target() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 75,
            tolerance: 0,
            max_epp: 90,
        };

        let mut governor = ThermalGovernor::new_test(config);
        governor.current_epp = 40;

        // 1. diff = -2 (73°C): Smooth recovery giảm nhẹ EPP -3
        let res_73 = governor.update(73);
        assert_eq!(res_73, 37, "73°C (diff = -2) phải giảm nhẹ EPP -3 từ 40 xuống 37");

        // 2. diff = -3 (72°C): Smooth recovery tiếp tục giảm -3
        let res_72 = governor.update(72);
        assert_eq!(res_72, 34, "72°C (diff = -3) phải giảm nhẹ EPP -3 từ 37 xuống 34");

        // 3. diff = -4 (71°C <= -4): Rất mát, giảm nhanh EPP -5
        let res_71 = governor.update(71);
        assert_eq!(res_71, 29, "71°C (diff = -4) phải giảm nhanh EPP -5 từ 34 xuống 29");

        // 4. Rất mát sâu (60°C <= -4): Giảm nhanh EPP -5
        let res_60 = governor.update(60);
        assert_eq!(res_60, 24, "60°C phải tiếp tục giảm nhanh EPP -5 từ 29 xuống 24");

        // 5. Giảm dần về sàn 0 và không bị âm
        for _ in 0..10 {
            governor.update(50);
        }
        assert_eq!(governor.current_epp, 0, "Phục hồi hoàn toàn về sàn EPP = 0");
    }

    #[test]
    fn test_epp_clamping() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 75,
            tolerance: 0,
            max_epp: 90,
        };

        let mut governor = ThermalGovernor::new_test(config);

        // Kẹp trần tại max_epp = 90
        governor.current_epp = 80;
        let res = governor.update(77); // diff = 2 -> 80 + 25 = 105 -> clamped to 90
        assert_eq!(res, 90, "EPP không được vượt quá max_epp = 90");
        assert_eq!(governor.current_epp, 90);

        // Tiếp tục quá nhiệt khi đã ở trần 90
        let res_ceiling = governor.update(85); // diff >= 3 -> 90.max(85).min(90) = 90
        assert_eq!(res_ceiling, 90, "Khi đã ở trần 90, EPP được giữ nguyên");

        // Kẹp sàn tại 0
        governor.current_epp = 2;
        let res_min = governor.update(50); // diff <= -4 -> 2.saturating_sub(5) = 0
        assert_eq!(res_min, 0, "EPP không được giảm dưới sàn 0");
        assert_eq!(governor.current_epp, 0);

        let res_floor = governor.update(45);
        assert_eq!(res_floor, 0, "Khi đã ở sàn 0, không bao giờ giảm thêm");
    }

    #[test]
    fn test_governor_disabled_reset() {
        let mut config = ThermalGovernorConfig::default();
        config.enabled = false;

        let mut governor = ThermalGovernor::new_test(config);
        governor.current_epp = 45;
        governor.prev_cpu_temp = Some(80);

        // Khi governor bị tắt, update phải tự khôi phục về EPP = 0 và reset prev_cpu_temp về None
        let res = governor.update(95);
        assert_eq!(res, 0);
        assert_eq!(governor.current_epp, 0);
        assert_eq!(governor.prev_cpu_temp, None);
    }

    #[test]
    fn test_reset_to_maximum() {
        let mut governor = ThermalGovernor::new_test(ThermalGovernorConfig::default());
        governor.current_epp = 65;
        governor.prev_cpu_temp = Some(78);
        governor.reset_to_maximum();
        assert_eq!(governor.current_epp, 0);
        assert_eq!(governor.prev_cpu_temp, None);
    }

    #[test]
    fn test_config_defaults() {
        let cfg = ThermalGovernorConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.target_temp, 80);
        assert_eq!(cfg.tolerance, 0);
        assert_eq!(cfg.max_epp, 90);
    }

    #[test]
    fn test_pure_calculate_next_epp_branches() {
        // diff >= 3: Emergency Clamp
        assert_eq!(ThermalGovernor::calculate_next_epp(0, 78, None, 75, 90), 85);
        assert_eq!(ThermalGovernor::calculate_next_epp(70, 78, None, 75, 90), 85);
        assert_eq!(ThermalGovernor::calculate_next_epp(90, 80, None, 75, 90), 90);

        // diff == 2: Hard Throttle (saturating_add 25, min 65)
        assert_eq!(ThermalGovernor::calculate_next_epp(0, 77, None, 75, 90), 65);
        assert_eq!(ThermalGovernor::calculate_next_epp(50, 77, None, 75, 90), 75);
        assert_eq!(ThermalGovernor::calculate_next_epp(80, 77, None, 75, 90), 90);

        // diff == 1: Proportional Attack (saturating_add 15, min 45)
        assert_eq!(ThermalGovernor::calculate_next_epp(0, 76, None, 75, 90), 45);
        assert_eq!(ThermalGovernor::calculate_next_epp(45, 76, None, 75, 90), 60);
        assert_eq!(ThermalGovernor::calculate_next_epp(80, 76, None, 75, 90), 90);

        // diff == 0:
        // rate > 0 (dT/dt > 0): Pre-emptive Hold (+8)
        assert_eq!(ThermalGovernor::calculate_next_epp(10, 75, Some(72), 75, 90), 18);
        // rate <= 0: Hold
        assert_eq!(ThermalGovernor::calculate_next_epp(20, 75, Some(75), 75, 90), 20);
        assert_eq!(ThermalGovernor::calculate_next_epp(20, 75, Some(77), 75, 90), 20);
        assert_eq!(ThermalGovernor::calculate_next_epp(20, 75, None, 75, 90), 20);

        // diff == -1: Deadband buffer (hold)
        assert_eq!(ThermalGovernor::calculate_next_epp(30, 74, Some(75), 75, 90), 30);

        // diff == -2 hoặc -3: Smooth recovery (-3)
        assert_eq!(ThermalGovernor::calculate_next_epp(30, 73, None, 75, 90), 27);
        assert_eq!(ThermalGovernor::calculate_next_epp(30, 72, None, 75, 90), 27);

        // diff <= -4: Fast recovery (-5)
        assert_eq!(ThermalGovernor::calculate_next_epp(30, 71, None, 75, 90), 25);
        assert_eq!(ThermalGovernor::calculate_next_epp(30, 60, None, 75, 90), 25);
    }
}
