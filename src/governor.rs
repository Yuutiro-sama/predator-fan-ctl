//! Module Vi điều khiển Năng lượng & Tần số Động (2-Phase Hysteresis Dynamic Power & Frequency Governor).
//!
//! Hiện thực hóa cơ chế **Khóa trần có điều kiện & Tự động xả xung khi mát**
//! (2-Phase Hysteresis Thermal Clamping) cho vi xử lý Intel Core i7-13700HX / 13650HX (Raptor Lake-HX):
//!
//! 1. **Trạng thái Mát / Bình thường (`ThermalCapPhase::NormalUnlocked`)**:
//!    - Mặc định xả toàn bộ xung: `PROCFREQMAX = 0` (All cores), `PROCFREQMAX_PCORE = 0` (P-cores).
//!    - `PROCTHROTTLEMAX = 100%` (Luôn giữ 100%, TUYỆT ĐỐI KHÔNG tắt Turbo Boost về 2.6 GHz).
//!    - `PERFEPP = 25` (Tối đa hóa độ nhạy và hiệu năng cho game).
//!    - Máy bung toàn bộ xung đỉnh 4.90 GHz khi môi trường mát / tải bình thường.
//!
//! 2. **Ngưỡng kích hoạt khóa trần (`Upper Trigger`)**:
//!    - Khi nhiệt độ `current_temp >= target_temp + 5` (ví dụ: set 80°C thì >= 85°C kích hoạt; set 75°C thì >= 80°C).
//!    - Bộ lọc Debounce 2 nhịp (2 ticks liên tiếp) để loại bỏ hoàn toàn nhiễu xung cảm biến DTS.
//!    - Chuyển sang `ThermalCapPhase::CappedCooling`:
//!      * `PROCFREQMAX = cap_mhz` (3600 MHz nếu target <= 75; 3800 MHz nếu target <= 82; 4100 MHz nếu target >= 83).
//!      * `PROCFREQMAX_PCORE = cap_mhz` (Đồng bộ nhân P-Cores).
//!      * `PROCTHROTTLEMAX = 100%` (Vẫn giữ Turbo Boost, không ngắt Turbo!).
//!      * `PERFEPP = 65` (Hạ tải năng lượng).
//!      * Công suất gói rơi ngay về 50W - 60W để dập nhiệt độ an toàn.
//!
//! 3. **Ngưỡng tự động xả xung (`Lower Release`)**:
//!    - Khi nhiệt độ đã hạ về `current_temp <= target_temp` (xác nhận sau 2 nhịp liên tiếp):
//!    - Lập tức chuyển về `ThermalCapPhase::NormalUnlocked`:
//!      * Xả lại xung: `PROCFREQMAX = 0`, `PROCFREQMAX_PCORE = 0`.
//!      * `PERFEPP = 25`.
//!      * Máy bung lại toàn bộ xung 4.90 GHz tức thì, mượt mà tuyệt đối không khựng khung hình.
//!
//! 4. **Vùng đệm (Hold Zone: `target < temp < target + 5`)**:
//!    - Giữ nguyên trạng thái hiện tại (đang Unlocked thì tiếp tục Unlocked; đang Capped thì tiếp tục hãm).
//!
//! 5. **Ghi Win32 Power API theo sườn (Edge-triggered)**:
//!    - Dirty check: Chỉ gọi `PowerWriteACValueIndex` / `PowerWriteDCValueIndex` và `PowerSetActiveScheme` khi
//!      chuyển trạng thái, triệt tiêu 100% hiện tượng spam trong vòng lặp telemetry.
//!    - An toàn RAII Fail-Safe: `impl Drop` và `reset_to_maximum` bắt buộc khôi phục `(0, 0, 100, 25)`.

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

/// GUID thiết lập tần số vi xử lý tối đa cho toàn bộ nhân CPU (Maximum Processor Frequency - All Cores / PROCFREQMAX)
/// Value: 75b0ae3f-bce0-45a7-8c89-c9611c25e100
/// - 0: Unlocked (Mở toàn bộ xung nhịp tối đa lên tới 4.90 GHz)
/// - cap_mhz (3600..4100 MHz): Khóa trần xung toàn bộ nhân để hạ công suất về 50W - 60W
pub const GUID_PROCFREQMAX: GUID = GUID {
    data1: 0x75b0ae3f,
    data2: 0xbce0,
    data3: 0x45a7,
    data4: [0x8c, 0x89, 0xc9, 0x61, 0x1c, 0x25, 0xe1, 0x00],
};
pub const GUID_PROCESSOR_FREQUENCY_LIMIT: GUID = GUID_PROCFREQMAX;

/// GUID thiết lập tần số vi xử lý tối đa cho riêng nhân hiệu năng cao P-Cores trên vi kiến trúc Hybrid
/// (Maximum Processor Frequency - Performance Cores / PROCFREQMAX_PCORE)
/// Value: 75b0ae3f-bce0-45a7-8c89-c9611c25e101
/// - 0: Unlocked (P-Cores boost tối đa 4.90 GHz)
/// - cap_mhz: Khóa trần xung P-Cores đồng bộ với All Cores
pub const GUID_PROCFREQMAX_PCORE: GUID = GUID {
    data1: 0x75b0ae3f,
    data2: 0xbce0,
    data3: 0x45a7,
    data4: [0x8c, 0x89, 0xc9, 0x61, 0x1c, 0x25, 0xe1, 0x01],
};
pub const GUID_PROCESSOR_FREQUENCY_LIMIT_1: GUID = GUID_PROCFREQMAX_PCORE;

/// GUID thiết lập phần trăm xung nhịp CPU tối đa (Maximum Processor State / PROCTHROTTLEMAX)
/// Value: bc5038f7-23e0-4960-96da-33abaf5935ec
/// - Luôn giữ 100%: Bảo toàn Turbo Boost tuyệt đối không ngắt về xung cơ bản 2.6 GHz.
pub const GUID_PROCTHROTTLEMAX: GUID = GUID {
    data1: 0xbc5038f7,
    data2: 0x23e0,
    data3: 0x4960,
    data4: [0x96, 0xda, 0x33, 0xab, 0xaf, 0x59, 0x35, 0xec],
};
pub const GUID_PROCESSOR_THROTTLE_MAXIMUM: GUID = GUID_PROCTHROTTLEMAX;

/// GUID thiết lập thiên hướng hiệu năng / năng lượng (Energy Performance Preference / PERFEPP)
/// Value: 36687f9e-e3a5-4dbf-b1dc-15eb381c6863
/// - 25: Unlocked (Tối đa hóa độ nhạy cho game, phản hồi tức thì)
/// - 65: Capped Cooling (Điều hòa xung nhịp, dập nhiệt độ và ổn định công suất)
pub const GUID_PERFEPP: GUID = GUID {
    data1: 0x36687f9e,
    data2: 0xe3a5,
    data3: 0x4dbf,
    data4: [0xb1, 0xdc, 0x15, 0xeb, 0x38, 0x1c, 0x68, 0x63],
};
pub const GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE: GUID = GUID_PERFEPP;

/// Trạng thái pha của bộ điều phối nhiệt độ và trần xung có điều kiện (2-Phase Hysteresis Clamping)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThermalCapPhase {
    /// Pha Mát / Bình thường: Mặc định xả toàn bộ xung (PROCFREQMAX = 0, PROCFREQMAX_PCORE = 0, PROCTHROTTLEMAX = 100%, PERFEPP = 25)
    NormalUnlocked,
    /// Pha Khóa trần làm mát: Khi nhiệt độ >= target + 5 (2 nhịp): (PROCFREQMAX = cap_mhz, PROCFREQMAX_PCORE = cap_mhz, PROCTHROTTLEMAX = 100%, PERFEPP = 65)
    CappedCooling,
}

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

    /// Giới hạn EPP tối đa - mặc định 90
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

/// Bộ điều phối nhiệt độ và tần số động 2 pha (2-Phase Hysteresis Dynamic Power & Frequency Governor).
/// Tự động xả toàn bộ xung nhịp khi mát (4.90 GHz) và khóa trần có điều kiện khi nhiệt độ vượt ngưỡng (>= Target + 5°C).
pub struct ThermalGovernor {
    /// Cấu hình hoạt động
    pub config: ThermalGovernorConfig,

    /// Pha hiện tại của bộ điều phối nhiệt độ (NormalUnlocked / CappedCooling)
    pub phase: ThermalCapPhase,

    /// Bộ đếm nhịp debounce (2 ticks liên tiếp để xác nhận chuyển pha)
    pub confirm_ticks: u8,

    /// Mức trần xung nhịp hiện tại đang áp dụng (0 = Unlocked 4.90 GHz, hoặc 3600..4100 MHz)
    pub current_cap_mhz: u32,

    /// Mức Throttle tối đa hiện tại (luôn giữ 100%)
    pub current_throttle_max: u32,

    /// Mức EPP hiện tại đang áp dụng (25 khi NormalUnlocked, 65 khi CappedCooling)
    pub current_epp: u32,

    /// Nhiệt độ CPU chu kỳ trước
    pub prev_cpu_temp: Option<u32>,

    /// Mức EPP gốc đọc từ hệ điều hành lúc khởi động để dự phòng
    pub original_epp: u32,

    /// Mức trần xung cuối cùng đã ghi thành công vào Windows Power Subsystem
    pub last_applied_cap_mhz: Option<u32>,

    /// Mức Throttle tối đa cuối cùng đã ghi thành công vào Windows Power Subsystem
    pub last_applied_throttle_max: Option<u32>,

    /// Mức EPP cuối cùng đã ghi thành công vào Windows Power Subsystem
    pub last_applied_epp: Option<u32>,

    /// GUID của Power Scheme đang hoạt động trên Windows
    pub active_scheme_guid: Option<GUID>,

    /// Cờ kiểm soát việc ghi trực tiếp vào phần cứng (mặc định: true, dùng false trong unit test)
    pub apply_to_hardware: bool,
}

pub type DynamicThermalCapController = ThermalGovernor;

impl ThermalGovernor {
    /// Khởi tạo ThermalGovernor với cấu hình chỉ định.
    /// Tự động kết nối với Win32 Power Management subsystem và kích hoạt trạng thái mặc định:
    /// NormalUnlocked (PROCFREQMAX = 0, PROCFREQMAX_PCORE = 0, PROCTHROTTLEMAX = 100%, PERFEPP = 25).
    pub fn new(config: ThermalGovernorConfig) -> Self {
        let (active_scheme_guid, original_epp) = Self::init_power_scheme();

        let mut governor = Self {
            config,
            phase: ThermalCapPhase::NormalUnlocked,
            confirm_ticks: 0,
            current_cap_mhz: 0,
            current_throttle_max: 100,
            current_epp: 25,
            prev_cpu_temp: None,
            original_epp,
            last_applied_cap_mhz: None,
            last_applied_throttle_max: None,
            last_applied_epp: None,
            active_scheme_guid,
            apply_to_hardware: true,
        };

        // Ghi áp dụng trạng thái ban đầu: NormalUnlocked (0, 0, 100, 25)
        governor.sync_hardware_profile(true);
        governor
    }

    /// Khởi tạo phiên bản kiểm thử không can thiệp trực tiếp vào thiết lập nguồn của hệ điều hành.
    pub fn new_test(config: ThermalGovernorConfig) -> Self {
        Self {
            config,
            phase: ThermalCapPhase::NormalUnlocked,
            confirm_ticks: 0,
            current_cap_mhz: 0,
            current_throttle_max: 100,
            current_epp: 25,
            prev_cpu_temp: None,
            original_epp: 50,
            last_applied_cap_mhz: Some(0),
            last_applied_throttle_max: Some(100),
            last_applied_epp: Some(25),
            active_scheme_guid: None,
            apply_to_hardware: false,
        }
    }

    /// Ánh xạ mức xung trần tương ứng với nhiệt độ mục tiêu:
    /// - target <= 75°C -> 3600 MHz
    /// - target <= 82°C (vd: 80°C) -> 3800 MHz
    /// - target >= 83°C (vd: 85°C) -> 4100 MHz
    #[inline]
    pub fn cap_mhz_for_target(target_temp: u32) -> u32 {
        if target_temp <= 75 {
            3600
        } else if target_temp <= 82 {
            3800
        } else {
            4100
        }
    }

    /// Trả về mức xung trần cap_mhz tương ứng với cấu hình mục tiêu hiện tại.
    #[inline]
    pub fn resolve_cap_mhz(&self) -> u32 {
        Self::cap_mhz_for_target(self.config.target_temp)
    }

    /// Trả về pha điều phối nhiệt độ hiện tại.
    #[inline]
    pub fn current_phase(&self) -> ThermalCapPhase {
        self.phase
    }

    /// Kiểm tra xem CPU có đang ở trạng thái khóa trần làm mát (CappedCooling) hay không.
    #[inline]
    pub fn is_capped(&self) -> bool {
        self.phase == ThermalCapPhase::CappedCooling
    }

    /// Tương thích ngược: Kiểm tra trạng thái hãm nhiệt độ.
    #[inline]
    pub fn is_turbo_cut(&self) -> bool {
        self.is_capped()
    }

    /// Trả về số nhịp xác nhận hiện tại của bộ lọc debounce.
    #[inline]
    pub fn confirm_ticks(&self) -> u8 {
        self.confirm_ticks
    }

    /// Hàm nội bộ đọc Active Power Scheme GUID và đọc EPP gốc từ Windows
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
                Some(&GUID_PERFEPP),
                &mut orig_val,
            );
            let orig = if read_res.0 == 0 && orig_val > 0 { orig_val } else { 50 };

            (Some(scheme), orig)
        }
    }

    /// Đánh giá trạng thái máy trạng thái trễ 2 pha (2-Phase Hysteresis FSM):
    /// - Khi NormalUnlocked: Kích hoạt CappedCooling khi `current_temp >= target + 5` (sau 2 nhịp debounce liên tiếp).
    /// - Khi CappedCooling: Tự động xả về NormalUnlocked khi `current_temp <= target` (sau 2 nhịp debounce liên tiếp).
    /// - Vùng Hold (`target < current_temp < target + 5`): Giữ nguyên trạng thái hiện tại.
    pub fn evaluate(&mut self, current_temp_c: f32) -> ThermalCapPhase {
        if !self.config.enabled {
            if self.phase != ThermalCapPhase::NormalUnlocked
                || self.last_applied_cap_mhz != Some(0)
                || self.last_applied_epp != Some(25)
            {
                self.phase = ThermalCapPhase::NormalUnlocked;
                self.confirm_ticks = 0;
                self.sync_hardware_profile(false);
            }
            return self.phase;
        }

        let target = self.config.target_temp as f32;
        let upper_trigger = target + 5.0; // Ngưỡng kích hoạt khóa trần (vd: 80 + 5 = 85°C)
        let lower_release = target;       // Ngưỡng tự động xả xung (vd: 80°C)

        match self.phase {
            ThermalCapPhase::NormalUnlocked => {
                if current_temp_c >= upper_trigger {
                    self.confirm_ticks += 1;
                    if self.confirm_ticks >= 2 {
                        self.phase = ThermalCapPhase::CappedCooling;
                        self.confirm_ticks = 0;
                        self.sync_hardware_profile(false);
                    }
                } else {
                    self.confirm_ticks = 0;
                }
            }
            ThermalCapPhase::CappedCooling => {
                if current_temp_c <= lower_release {
                    self.confirm_ticks += 1;
                    if self.confirm_ticks >= 2 {
                        self.phase = ThermalCapPhase::NormalUnlocked;
                        self.confirm_ticks = 0;
                        self.sync_hardware_profile(false);
                    }
                } else {
                    self.confirm_ticks = 0;
                }
            }
        }

        // Tự động đồng bộ lại nếu config target_temp thay đổi làm thay đổi resolve_cap_mhz khi đang Capped
        if self.phase == ThermalCapPhase::CappedCooling
            && self.last_applied_cap_mhz != Some(self.resolve_cap_mhz())
        {
            self.sync_hardware_profile(false);
        }

        self.phase
    }

    /// Cập nhật cấu hình bộ điều phối nhiệt độ và tự động đồng bộ lại phần cứng.
    pub fn set_config(&mut self, enabled: bool, new_target_temp_c: u32) {
        self.config.enabled = enabled;
        self.config.target_temp = new_target_temp_c;
        self.confirm_ticks = 0;

        if !self.config.enabled {
            self.phase = ThermalCapPhase::NormalUnlocked;
        }
        self.sync_hardware_profile(false);
    }

    /// Cập nhật vòng lặp điều khiển nhiệt độ với giá trị nhiệt độ CPU hiện tại (°C).
    /// Áp dụng tối ưu hóa Win32 Power API (chỉ ghi khi thông số thay đổi).
    /// Trả về mức EPP (25 hoặc 65) sau khi tính toán.
    pub fn update(&mut self, current_cpu_temp: u32) -> u32 {
        let _ = self.evaluate(current_cpu_temp as f32);
        self.prev_cpu_temp = Some(current_cpu_temp);
        self.current_epp
    }

    /// Đồng bộ trạng thái cấu hình năng lượng xuống hệ điều hành Windows qua Win32 Power API.
    ///
    /// Áp dụng kỹ thuật Edge-Triggered Dirty Check:
    /// Chỉ gọi `PowerWriteACValueIndex` / `PowerWriteDCValueIndex` và `PowerSetActiveScheme`
    /// khi giá trị tham số thực sự thay đổi hoặc khi có cờ `force == true`.
    pub fn sync_hardware_profile(&mut self, force: bool) -> bool {
        let (target_cap_mhz, target_throttle_max, target_epp) = match self.phase {
            ThermalCapPhase::NormalUnlocked => (0, 100, 25),
            ThermalCapPhase::CappedCooling => (self.resolve_cap_mhz(), 100, 65),
        };

        self.current_cap_mhz = target_cap_mhz;
        self.current_throttle_max = target_throttle_max;
        self.current_epp = target_epp;

        // Dirty Check: Bỏ qua gọi Win32 Power API nếu giá trị không đổi
        if !force
            && self.last_applied_cap_mhz == Some(target_cap_mhz)
            && self.last_applied_throttle_max == Some(target_throttle_max)
            && self.last_applied_epp == Some(target_epp)
        {
            return true;
        }

        if !self.apply_to_hardware {
            self.last_applied_cap_mhz = Some(target_cap_mhz);
            self.last_applied_throttle_max = Some(target_throttle_max);
            self.last_applied_epp = Some(target_epp);
            return true;
        }

        let success = Self::write_windows_power_ceiling(
            self.active_scheme_guid.as_ref(),
            target_cap_mhz,
            target_throttle_max,
            target_epp,
        );

        if success {
            self.last_applied_cap_mhz = Some(target_cap_mhz);
            self.last_applied_throttle_max = Some(target_throttle_max);
            self.last_applied_epp = Some(target_epp);
        }

        success
    }

    /// Ghi đồng bộ 4 tham số nguồn (AC & DC) vào Active Power Scheme và kích hoạt lại scheme:
    /// 1. `GUID_PROCFREQMAX` (All Cores) = cap_mhz
    /// 2. `GUID_PROCFREQMAX_PCORE` (P-Cores) = cap_mhz
    /// 3. `GUID_PROCTHROTTLEMAX` = throttle_max (100)
    /// 4. `GUID_PERFEPP` = epp (25 hoặc 65)
    /// 5. `PowerSetActiveScheme(NULL, scheme)`
    pub fn write_windows_power_ceiling(
        active_scheme: Option<&GUID>,
        cap_mhz: u32,
        throttle_max: u32,
        epp: u32,
    ) -> bool {
        unsafe {
            let scheme = match active_scheme {
                Some(s) => *s,
                None => {
                    let mut p_guid: *mut GUID = std::ptr::null_mut();
                    let res = PowerGetActiveScheme(None, &mut p_guid);
                    if res.0 != 0 || p_guid.is_null() {
                        return false;
                    }
                    let s = *p_guid;
                    let _ = LocalFree(HLOCAL(p_guid as _));
                    s
                }
            };

            // 1. GUID_PROCFREQMAX (All cores) = cap_mhz
            let _ = PowerWriteACValueIndex(
                None,
                &scheme,
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PROCFREQMAX),
                cap_mhz,
            );
            let _ = PowerWriteDCValueIndex(
                None,
                &scheme,
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PROCFREQMAX),
                cap_mhz,
            );

            // 2. GUID_PROCFREQMAX_PCORE (P-cores) = cap_mhz
            let _ = PowerWriteACValueIndex(
                None,
                &scheme,
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PROCFREQMAX_PCORE),
                cap_mhz,
            );
            let _ = PowerWriteDCValueIndex(
                None,
                &scheme,
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PROCFREQMAX_PCORE),
                cap_mhz,
            );

            // 3. GUID_PROCTHROTTLEMAX = throttle_max (100)
            let _ = PowerWriteACValueIndex(
                None,
                &scheme,
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PROCTHROTTLEMAX),
                throttle_max,
            );
            let _ = PowerWriteDCValueIndex(
                None,
                &scheme,
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PROCTHROTTLEMAX),
                throttle_max,
            );

            // 4. GUID_PERFEPP = epp
            let _ = PowerWriteACValueIndex(
                None,
                &scheme,
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PERFEPP),
                epp,
            );
            let _ = PowerWriteDCValueIndex(
                None,
                &scheme,
                Some(&GUID_PROCESSOR_SETTINGS_SUBGROUP),
                Some(&GUID_PERFEPP),
                epp,
            );

            // 5. Kích hoạt Active Scheme
            let res = PowerSetActiveScheme(None, Some(&scheme));
            res.0 == 0
        }
    }

    /// Tương thích ngược: Áp dụng giá trị cài đặt nguồn thủ công.
    pub fn apply_power_settings(&mut self, throttle_max: u32, epp_val: u32) -> bool {
        self.current_throttle_max = throttle_max;
        self.current_epp = epp_val;
        if !self.apply_to_hardware {
            self.last_applied_throttle_max = Some(throttle_max);
            self.last_applied_epp = Some(epp_val);
            return true;
        }
        Self::write_windows_power_ceiling(
            self.active_scheme_guid.as_ref(),
            self.current_cap_mhz,
            throttle_max,
            epp_val,
        )
    }

    /// Tương thích ngược: Áp dụng giá trị EPP với mức Throttle hiện tại.
    pub fn apply_epp(&mut self, epp_val: u32) -> bool {
        self.apply_power_settings(self.current_throttle_max, epp_val)
    }

    /// Khôi phục trạng thái vi xử lý về mức an toàn tối đa:
    /// - PROCFREQMAX = 0 (Xả toàn bộ xung nhịp lên tới 4.90 GHz)
    /// - PROCFREQMAX_PCORE = 0
    /// - PROCTHROTTLEMAX = 100%
    /// - PERFEPP = 25 (Độ nhạy tối đa)
    pub fn reset_to_maximum(&mut self) {
        self.phase = ThermalCapPhase::NormalUnlocked;
        self.confirm_ticks = 0;
        self.prev_cpu_temp = None;
        self.current_cap_mhz = 0;
        self.current_throttle_max = 100;
        self.current_epp = 25;

        if self.apply_to_hardware {
            Self::write_windows_power_ceiling(
                self.active_scheme_guid.as_ref(),
                0,
                100,
                25,
            );
        }

        self.last_applied_cap_mhz = Some(0);
        self.last_applied_throttle_max = Some(100);
        self.last_applied_epp = Some(25);
    }

    /// Khôi phục mức cài đặt về giá trị an toàn ban đầu.
    pub fn reset_to_original(&mut self) {
        self.reset_to_maximum();
    }
}

impl Drop for ThermalGovernor {
    /// Khi ThermalGovernor bị giải phóng bộ nhớ (Drop), bắt buộc khôi phục toàn bộ thiết lập về:
    /// PROCFREQMAX = 0, PROCFREQMAX_PCORE = 0, PROCTHROTTLEMAX = 100%, PERFEPP = 25
    /// để đảm bảo máy tính không bị kẹt trần xung trong Windows Registry sau khi thoát ứng dụng.
    fn drop(&mut self) {
        if self.apply_to_hardware {
            Self::write_windows_power_ceiling(
                self.active_scheme_guid.as_ref(),
                0,
                100,
                25,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normal_unlocked_when_cool() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 80,
            tolerance: 0,
            max_epp: 90,
        };
        let mut governor = ThermalGovernor::new_test(config);

        // Ban đầu máy ở trạng thái NormalUnlocked
        assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked);
        assert_eq!(governor.current_cap_mhz, 0);
        assert_eq!(governor.current_epp, 25);
        assert_eq!(governor.current_throttle_max, 100);
        assert!(!governor.is_capped());

        // Chạy qua dải nhiệt độ mát (50°C, 60°C, 75°C, 80°C)
        for temp in [50, 60, 75, 80] {
            let epp = governor.update(temp);
            assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked);
            assert_eq!(governor.current_cap_mhz, 0);
            assert_eq!(governor.current_epp, 25);
            assert_eq!(epp, 25);
            assert!(!governor.is_capped());
        }
    }

    #[test]
    fn test_debounce_filter_rejects_single_tick_spike() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 80,
            tolerance: 0,
            max_epp: 90,
        };
        let mut governor = ThermalGovernor::new_test(config);

        // Nhịp 1: Nhiệt độ vọt lên 86°C (>= Target + 5 = 85°C) -> confirm_ticks = 1, vẫn Unlocked
        governor.update(86);
        assert_eq!(governor.confirm_ticks(), 1);
        assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked);
        assert_eq!(governor.current_cap_mhz, 0);
        assert_eq!(governor.current_epp, 25);

        // Nhịp 2: Nhiệt độ hạ xuống 84°C (< 85°C) -> Reset confirm_ticks về 0, giữ Unlocked
        governor.update(84);
        assert_eq!(governor.confirm_ticks(), 0);
        assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked);
        assert_eq!(governor.current_cap_mhz, 0);
        assert_eq!(governor.current_epp, 25);
    }

    #[test]
    fn test_upper_trigger_activation_two_ticks() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 80,
            tolerance: 0,
            max_epp: 90,
        };
        let mut governor = ThermalGovernor::new_test(config);

        // Nhịp 1: 85°C (chạm ngưỡng Target + 5) -> confirm_ticks = 1, vẫn Unlocked
        governor.update(85);
        assert_eq!(governor.confirm_ticks(), 1);
        assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked);

        // Nhịp 2: Tiếp tục 85°C (đạt 2 nhịp liên tiếp) -> Kích hoạt CappedCooling!
        let epp = governor.update(85);
        assert_eq!(governor.confirm_ticks(), 0);
        assert_eq!(governor.current_phase(), ThermalCapPhase::CappedCooling);
        assert_eq!(governor.current_cap_mhz, 3800); // Target 80°C -> 3800 MHz
        assert_eq!(governor.current_epp, 65);
        assert_eq!(governor.current_throttle_max, 100);
        assert_eq!(epp, 65);
        assert!(governor.is_capped());
    }

    #[test]
    fn test_hold_zone_retention() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 80,
            tolerance: 0,
            max_epp: 90,
        };
        let mut governor = ThermalGovernor::new_test(config);

        // 1. Kích hoạt CappedCooling bằng 2 nhịp 85°C
        governor.update(85);
        governor.update(85);
        assert_eq!(governor.current_phase(), ThermalCapPhase::CappedCooling);

        // 2. Nhiệt độ trong Hold Zone (81°C, 82°C, 83°C, 84°C): Duy trì CappedCooling
        for temp in [84, 83, 82, 81] {
            governor.update(temp);
            assert_eq!(governor.current_phase(), ThermalCapPhase::CappedCooling);
            assert_eq!(governor.current_cap_mhz, 3800);
            assert_eq!(governor.current_epp, 65);
            assert_eq!(governor.confirm_ticks(), 0);
        }

        // 3. Chạm 80°C nhịp 1 (confirm_ticks = 1), nhưng chưa xả vì mới 1 nhịp
        governor.update(80);
        assert_eq!(governor.confirm_ticks(), 1);
        assert_eq!(governor.current_phase(), ThermalCapPhase::CappedCooling);

        // 4. Bật lại 82°C (quay lại Hold Zone) -> Reset confirm_ticks về 0, tiếp tục duy trì CappedCooling
        governor.update(82);
        assert_eq!(governor.confirm_ticks(), 0);
        assert_eq!(governor.current_phase(), ThermalCapPhase::CappedCooling);
    }

    #[test]
    fn test_lower_release_two_ticks_and_instant_unlocked() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 80,
            tolerance: 0,
            max_epp: 90,
        };
        let mut governor = ThermalGovernor::new_test(config);

        // Đẩy vào trạng thái CappedCooling
        governor.update(86);
        governor.update(86);
        assert_eq!(governor.current_phase(), ThermalCapPhase::CappedCooling);
        assert_eq!(governor.current_cap_mhz, 3800);

        // Nhịp 1 làm mát: 80°C (<= Target 80°C) -> confirm_ticks = 1, vẫn Capped
        governor.update(80);
        assert_eq!(governor.confirm_ticks(), 1);
        assert_eq!(governor.current_phase(), ThermalCapPhase::CappedCooling);

        // Nhịp 2 làm mát: 79°C (<= Target 80°C) -> Xác nhận đủ 2 nhịp! LẬP TỨC XẢ XUNG VỀ UNLOCKED
        let epp = governor.update(79);
        assert_eq!(governor.confirm_ticks(), 0);
        assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked);
        assert_eq!(governor.current_cap_mhz, 0); // Xả xung 4.90 GHz
        assert_eq!(governor.current_epp, 25);
        assert_eq!(governor.current_throttle_max, 100);
        assert_eq!(epp, 25);
        assert!(!governor.is_capped());

        // Tiếp tục mát về 75°C: Duy trì Unlocked
        governor.update(75);
        assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked);
        assert_eq!(governor.current_cap_mhz, 0);
        assert_eq!(governor.current_epp, 25);
    }

    #[test]
    fn test_hold_zone_from_unlocked_does_not_cap_early() {
        let config = ThermalGovernorConfig {
            enabled: true,
            target_temp: 80,
            tolerance: 0,
            max_epp: 90,
        };
        let mut governor = ThermalGovernor::new_test(config);

        // Tăng từ 78°C lên 81°C, 82°C, 83°C, 84°C (nằm trong Hold Zone [80, 85))
        for temp in [78, 81, 82, 83, 84] {
            governor.update(temp);
            assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked, "Tại {}°C không được bóp xung sớm", temp);
            assert_eq!(governor.current_cap_mhz, 0);
            assert_eq!(governor.current_epp, 25);
            assert_eq!(governor.confirm_ticks(), 0);
        }
    }

    #[test]
    fn test_cap_frequency_mapping_by_target() {
        // Target <= 75 -> 3600 MHz
        assert_eq!(ThermalGovernor::cap_mhz_for_target(70), 3600);
        assert_eq!(ThermalGovernor::cap_mhz_for_target(75), 3600);

        // Target <= 82 -> 3800 MHz
        assert_eq!(ThermalGovernor::cap_mhz_for_target(76), 3800);
        assert_eq!(ThermalGovernor::cap_mhz_for_target(80), 3800);
        assert_eq!(ThermalGovernor::cap_mhz_for_target(82), 3800);

        // Target >= 83 -> 4100 MHz
        assert_eq!(ThermalGovernor::cap_mhz_for_target(83), 4100);
        assert_eq!(ThermalGovernor::cap_mhz_for_target(85), 4100);
        assert_eq!(ThermalGovernor::cap_mhz_for_target(90), 4100);
    }

    #[test]
    fn test_power_api_caching_and_avoiding_spam() {
        let config = ThermalGovernorConfig::default();
        let mut governor = ThermalGovernor::new_test(config);

        // Đã đồng bộ ban đầu: (0, 100, 25)
        assert_eq!(governor.last_applied_cap_mhz, Some(0));
        assert_eq!(governor.last_applied_throttle_max, Some(100));
        assert_eq!(governor.last_applied_epp, Some(25));

        // Gọi sync_hardware_profile(false) khi chưa đổi phase: trả về true ngay mà không cần ghi lại
        assert!(governor.sync_hardware_profile(false));
        assert_eq!(governor.last_applied_cap_mhz, Some(0));

        // Khi chuyển sang Capped: cập nhật cache
        governor.phase = ThermalCapPhase::CappedCooling;
        assert!(governor.sync_hardware_profile(false));
        assert_eq!(governor.last_applied_cap_mhz, Some(3800));
        assert_eq!(governor.last_applied_epp, Some(65));

        // Gọi lại không thay đổi: idempotent
        assert!(governor.sync_hardware_profile(false));
    }

    #[test]
    fn test_fail_safe_cleanup_and_drop() {
        let mut governor = ThermalGovernor::new_test(ThermalGovernorConfig::default());
        governor.phase = ThermalCapPhase::CappedCooling;
        governor.current_cap_mhz = 3800;
        governor.current_epp = 65;
        governor.confirm_ticks = 1;

        // Gọi reset_to_maximum: bắt buộc khôi phục NormalUnlocked (0, 0, 100, 25)
        governor.reset_to_maximum();
        assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked);
        assert_eq!(governor.current_cap_mhz, 0);
        assert_eq!(governor.current_throttle_max, 100);
        assert_eq!(governor.current_epp, 25);
        assert_eq!(governor.confirm_ticks(), 0);
        assert_eq!(governor.last_applied_cap_mhz, Some(0));
        assert_eq!(governor.last_applied_throttle_max, Some(100));
        assert_eq!(governor.last_applied_epp, Some(25));

        // Khi governor bị tắt (disabled): update() tự động đưa về NormalUnlocked
        governor.config.enabled = false;
        governor.phase = ThermalCapPhase::CappedCooling;
        let epp = governor.update(90);
        assert_eq!(epp, 25);
        assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked);
        assert_eq!(governor.current_cap_mhz, 0);
        assert_eq!(governor.current_epp, 25);
    }

    #[test]
    fn test_set_config_dynamic_switching() {
        let mut governor = ThermalGovernor::new_test(ThermalGovernorConfig::default());

        // Đang Capped ở target 80°C (cap 3800 MHz)
        governor.update(85);
        governor.update(85);
        assert_eq!(governor.current_phase(), ThermalCapPhase::CappedCooling);
        assert_eq!(governor.current_cap_mhz, 3800);

        // Người dùng đổi target sang 75°C: Cap tự động điều chỉnh về 3600 MHz
        governor.set_config(true, 75);
        assert_eq!(governor.current_cap_mhz, 3600);
        assert_eq!(governor.last_applied_cap_mhz, Some(3600));

        // Người dùng tắt tính năng governor: Lập tức xả về NormalUnlocked
        governor.set_config(false, 75);
        assert_eq!(governor.current_phase(), ThermalCapPhase::NormalUnlocked);
        assert_eq!(governor.current_cap_mhz, 0);
        assert_eq!(governor.current_epp, 25);
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
