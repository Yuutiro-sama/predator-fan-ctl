use crate::curve::CurvePoint;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Xác định đường dẫn file cấu hình `config.toml`:
/// 1. Ưu tiên tìm file `config.toml` nằm cùng thư mục với file `.exe` (`std::env::current_exe()?.parent().unwrap().join("config.toml")`).
/// 2. Nếu không tìm thấy, fallback sang thư mục làm việc hiện tại (`config.toml`).
/// 3. Nếu chưa tồn tại ở cả 2 nơi, ưu tiên đường dẫn cùng thư mục `.exe` để tạo mới đúng vị trí.
///
/// Đảm bảo khi người dùng mở ứng dụng từ Shortcut Desktop, Start Menu hoặc Windows Search (Working Directory khác),
/// ứng dụng vẫn luôn tìm thấy và tải đúng file cấu hình.
pub fn get_config_path() -> PathBuf {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let exe_config = parent.join("config.toml");
            if exe_config.exists() {
                return exe_config;
            }
        }
    }

    let cwd_config = PathBuf::from("config.toml");
    if cwd_config.exists() {
        return cwd_config;
    }

    // Nếu chưa tồn tại ở đâu, ưu tiên tạo cùng thư mục với .exe
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            return parent.join("config.toml");
        }
    }

    cwd_config
}

/// Cấu hình các địa chỉ thanh ghi ACPI Embedded Controller (sử dụng khi `backend = "ec"`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EcRegistersConfig {
    #[serde(default = "default_cpu_fan_mode_reg")]
    pub cpu_fan_mode_reg: u8,

    #[serde(default = "default_cpu_fan_speed_reg")]
    pub cpu_fan_speed_reg: u8,

    #[serde(default = "default_gpu_fan_mode_reg")]
    pub gpu_fan_mode_reg: u8,

    #[serde(default = "default_gpu_fan_speed_reg")]
    pub gpu_fan_speed_reg: u8,
}

impl Default for EcRegistersConfig {
    fn default() -> Self {
        Self {
            cpu_fan_mode_reg: default_cpu_fan_mode_reg(),
            cpu_fan_speed_reg: default_cpu_fan_speed_reg(),
            gpu_fan_mode_reg: default_gpu_fan_mode_reg(),
            gpu_fan_speed_reg: default_gpu_fan_speed_reg(),
        }
    }
}

const fn default_cpu_fan_mode_reg() -> u8 {
    0x21
}

const fn default_cpu_fan_speed_reg() -> u8 {
    0x37
}

const fn default_gpu_fan_mode_reg() -> u8 {
    0x22
}

const fn default_gpu_fan_speed_reg() -> u8 {
    0x3A
}

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

pub const fn default_min_throttle() -> u32 {
    55
}

/// Cấu hình bộ điều phối nhiệt độ CPU mục tiêu (Dynamic Closed-Loop Thermal Governor qua Intel Speed Shift EPP)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThermalGovernorConfig {
    #[serde(default = "default_governor_enabled")]
    pub enabled: bool,
    #[serde(default = "default_target_temp")]
    pub target_cpu_temp: u32,
    #[serde(default = "default_tolerance")]
    pub tolerance: u32,
    #[serde(default = "default_max_epp")]
    pub max_epp: u32,
    #[serde(default = "default_min_throttle")]
    pub min_throttle_pct: u32,
}

impl Default for ThermalGovernorConfig {
    fn default() -> Self {
        Self {
            enabled: default_governor_enabled(),
            target_cpu_temp: default_target_temp(),
            tolerance: default_tolerance(),
            max_epp: default_max_epp(),
            min_throttle_pct: default_min_throttle(),
        }
    }
}

const fn default_poll_interval_ms() -> u64 {
    1000
}

fn default_backend() -> String {
    "wmi".to_string()
}

const fn default_manage_predator_service() -> bool {
    true
}

fn default_service_name() -> String {
    "PredatorSenseService".to_string()
}

fn default_cpu_curve() -> Vec<CurvePoint> {
    vec![
        CurvePoint::new(40, 20),
        CurvePoint::new(60, 50),
        CurvePoint::new(75, 80),
        CurvePoint::new(85, 100),
    ]
}

fn default_gpu_curve() -> Vec<CurvePoint> {
    vec![
        CurvePoint::new(45, 25),
        CurvePoint::new(65, 60),
        CurvePoint::new(80, 100),
    ]
}

/// Cấu hình toàn diện của ứng dụng Predator Fan Controller.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,

    #[serde(default = "default_backend")]
    pub backend: String,

    #[serde(default = "default_manage_predator_service")]
    pub manage_predator_service: bool,

    #[serde(default = "default_service_name")]
    pub service_name: String,

    #[serde(default)]
    pub ec_registers: EcRegistersConfig,

    #[serde(default = "default_cpu_curve")]
    pub cpu_curve: Vec<CurvePoint>,

    #[serde(default = "default_gpu_curve")]
    pub gpu_curve: Vec<CurvePoint>,

    #[serde(default)]
    pub thermal_governor: ThermalGovernorConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_interval_ms: default_poll_interval_ms(),
            backend: default_backend(),
            manage_predator_service: default_manage_predator_service(),
            service_name: default_service_name(),
            ec_registers: EcRegistersConfig::default(),
            cpu_curve: default_cpu_curve(),
            gpu_curve: default_gpu_curve(),
            thermal_governor: ThermalGovernorConfig::default(),
        }
    }
}

impl Config {
    /// Phân tích chuỗi TOML và chuẩn hóa đường cong nhiệt độ (tự động sắp xếp tăng dần).
    pub fn parse_str(content: &str) -> Result<Self, String> {
        let mut config: Config = toml::from_str(content)
            .map_err(|e| format!("Lỗi cú pháp TOML: {}", e))?;

        config.validate_and_sanitize();
        Ok(config)
    }

    /// Chuẩn hóa dữ liệu: sắp xếp đường cong nhiệt độ tăng dần và giới hạn tốc độ <= 100%.
    pub fn validate_and_sanitize(&mut self) {
        if self.poll_interval_ms < 100 {
            self.poll_interval_ms = 100; // Giới hạn chu kỳ tối thiểu để chống tràn CPU
        }

        let backend_lower = self.backend.to_lowercase();
        if backend_lower != "wmi" && backend_lower != "ec" {
            self.backend = "wmi".to_string();
        } else {
            self.backend = backend_lower;
        }

        // Sắp xếp tăng dần theo nhiệt độ
        self.cpu_curve.sort_by_key(|p| p.temp);
        for p in &mut self.cpu_curve {
            p.speed = p.speed.min(100);
        }

        self.gpu_curve.sort_by_key(|p| p.temp);
        for p in &mut self.gpu_curve {
            p.speed = p.speed.min(100);
        }

        // Đảm bảo không rỗng bằng fallback an toàn
        if self.cpu_curve.is_empty() {
            self.cpu_curve = default_cpu_curve();
        }
        if self.gpu_curve.is_empty() {
            self.gpu_curve = default_gpu_curve();
        }

        self.thermal_governor.target_cpu_temp = self.thermal_governor.target_cpu_temp.clamp(50, 100);
        self.thermal_governor.tolerance = self.thermal_governor.tolerance.clamp(1, 10);
        self.thermal_governor.max_epp = self.thermal_governor.max_epp.clamp(10, 100);
        self.thermal_governor.min_throttle_pct = self.thermal_governor.min_throttle_pct.clamp(20, 100);
    }

    /// Tải cấu hình từ đường dẫn chỉ định.
    /// - Nếu file chưa tồn tại: tự động tạo file mẫu mặc định và trả về `Config::default()`.
    /// - Nếu file bị lỗi cú pháp: trả về `Config::default()` an toàn (fail-safe fallback).
    pub fn load_or_default<P: AsRef<Path>>(path: P) -> Self {
        let path_ref = path.as_ref();
        if !path_ref.exists() {
            let default_cfg = Self::default();
            if let Ok(toml_str) = default_cfg.to_toml_string() {
                let _ = fs::write(path_ref, toml_str);
            }
            return default_cfg;
        }

        match fs::read_to_string(path_ref) {
            Ok(content) => match Self::parse_str(&content) {
                Ok(cfg) => cfg,
                Err(err) => {
                    eprintln!(
                        "[Config] Cảnh báo: Lỗi đọc file '{}': {}. Sử dụng cấu hình mặc định an toàn.",
                        path_ref.display(),
                        err
                    );
                    Self::default()
                }
            },
            Err(err) => {
                eprintln!(
                    "[Config] Không thể mở file '{}': {}. Sử dụng cấu hình mặc định an toàn.",
                    path_ref.display(),
                    err
                );
                Self::default()
            }
        }
    }

    /// Hot-reload cấu hình từ file `config.toml` (hoặc đường dẫn được cung cấp).
    ///
    /// Cơ chế An toàn (Fail-Safe):
    /// - Nếu file không đọc được hoặc có lỗi cú pháp, hàm trả về `Err(msg)` và **GIỮ NGUYÊN** cấu hình đang chạy,
    ///   tuyệt đối không làm gián đoạn vòng lặp điều khiển quạt hiện tại.
    pub fn reload_from<P: AsRef<Path>>(&mut self, path: P) -> Result<(), String> {
        let path_ref = path.as_ref();
        let content = fs::read_to_string(path_ref)
            .map_err(|e| format!("Không thể đọc file '{}': {}", path_ref.display(), e))?;

        let new_config = Self::parse_str(&content)?;
        *self = new_config;
        Ok(())
    }

    /// Hot-reload tiện lợi từ file cấu hình tự động nhận diện (`get_config_path()`).
    pub fn reload(&mut self) -> Result<(), String> {
        self.reload_from(get_config_path())
    }

    /// Xuất cấu hình ra chuỗi định dạng TOML chuẩn.
    pub fn to_toml_string(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("Lỗi serialize TOML: {}", e))
    }

    /// Lưu cấu hình ra file chỉ định.
    pub fn save_to<P: AsRef<Path>>(&self, path: P) -> Result<(), String> {
        let toml_str = self.to_toml_string()?;
        fs::write(path.as_ref(), toml_str)
            .map_err(|e| format!("Không thể ghi file '{}': {}", path.as_ref().display(), e))
    }

    /// Lưu cấu hình vào file cấu hình tự động nhận diện (`get_config_path()`).
    pub fn save(&self) -> Result<(), String> {
        self.save_to(get_config_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.poll_interval_ms, 1000);
        assert_eq!(cfg.backend, "wmi");
        assert!(cfg.manage_predator_service);
        assert_eq!(cfg.service_name, "PredatorSenseService");
        assert_eq!(cfg.ec_registers.cpu_fan_mode_reg, 0x21);
        assert_eq!(cfg.ec_registers.cpu_fan_speed_reg, 0x37);
        assert_eq!(cfg.ec_registers.gpu_fan_mode_reg, 0x22);
        assert_eq!(cfg.ec_registers.gpu_fan_speed_reg, 0x3A);
        assert_eq!(cfg.cpu_curve.len(), 4);
        assert_eq!(cfg.gpu_curve.len(), 3);
        assert!(cfg.thermal_governor.enabled);
        assert_eq!(cfg.thermal_governor.target_cpu_temp, 80);
        assert_eq!(cfg.thermal_governor.tolerance, 3);
        assert_eq!(cfg.thermal_governor.max_epp, 50);
        assert_eq!(cfg.thermal_governor.min_throttle_pct, 55);
    }

    #[test]
    fn test_thermal_governor_parse_and_sanitize() {
        let toml_data = r#"
[thermal_governor]
enabled = false
target_cpu_temp = 120
tolerance = 0
max_epp = 150
min_throttle_pct = 10
"#;
        let cfg = Config::parse_str(toml_data).expect("Phải parse thành công");
        assert!(!cfg.thermal_governor.enabled);
        assert_eq!(cfg.thermal_governor.target_cpu_temp, 100); // clamped to 100
        assert_eq!(cfg.thermal_governor.tolerance, 1); // clamped to 1
        assert_eq!(cfg.thermal_governor.max_epp, 100); // clamped to 100
        assert_eq!(cfg.thermal_governor.min_throttle_pct, 20); // clamped to 20
    }

    #[test]
    fn test_parse_valid_toml() {
        let toml_data = r#"
poll_interval_ms = 500
backend = "ec"
manage_predator_service = false
service_name = "AcerService"

[ec_registers]
cpu_fan_mode_reg = 0x20
cpu_fan_speed_reg = 0x30
gpu_fan_mode_reg = 0x24
gpu_fan_speed_reg = 0x3B

[[cpu_curve]]
temp = 50
speed = 30

[[cpu_curve]]
temp = 80
speed = 90

[[gpu_curve]]
temp = 60
speed = 70
"#;
        let cfg = Config::parse_str(toml_data).expect("Phải parse thành công");
        assert_eq!(cfg.poll_interval_ms, 500);
        assert_eq!(cfg.backend, "ec");
        assert!(!cfg.manage_predator_service);
        assert_eq!(cfg.service_name, "AcerService");
        assert_eq!(cfg.ec_registers.cpu_fan_mode_reg, 0x20);
        assert_eq!(cfg.cpu_curve.len(), 2);
        assert_eq!(cfg.cpu_curve[0], CurvePoint::new(50, 30));
        assert_eq!(cfg.cpu_curve[1], CurvePoint::new(80, 90));
        assert_eq!(cfg.gpu_curve.len(), 1);
        assert_eq!(cfg.gpu_curve[0], CurvePoint::new(60, 70));
    }

    #[test]
    fn test_parse_invalid_toml() {
        let broken = r#"
poll_interval_ms = "chữ thay vì số"
backend = 123
"#;
        let result = Config::parse_str(broken);
        assert!(result.is_err());
    }

    #[test]
    fn test_curve_auto_sorting_on_parse() {
        let toml_data = r#"
[[cpu_curve]]
temp = 85
speed = 100

[[cpu_curve]]
temp = 40
speed = 20

[[cpu_curve]]
temp = 60
speed = 50
"#;
        let cfg = Config::parse_str(toml_data).expect("Parse thành công");
        assert_eq!(cfg.cpu_curve[0].temp, 40);
        assert_eq!(cfg.cpu_curve[1].temp, 60);
        assert_eq!(cfg.cpu_curve[2].temp, 85);
    }

    #[test]
    fn test_hot_reload_success_and_failure_preservation() {
        let initial_toml = r#"
poll_interval_ms = 800
backend = "wmi"
"#;
        let mut cfg = Config::parse_str(initial_toml).unwrap();
        assert_eq!(cfg.poll_interval_ms, 800);

        // Tạo temp file test
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_config_predator.toml");

        // Ghi cấu hình mới
        let updated_toml = r#"
poll_interval_ms = 1500
backend = "ec"
"#;
        fs::write(&test_file, updated_toml).unwrap();

        // Reload thành công
        let reload_res = cfg.reload_from(&test_file);
        assert!(reload_res.is_ok());
        assert_eq!(cfg.poll_interval_ms, 1500);
        assert_eq!(cfg.backend, "ec");

        // Ghi file lỗi cú pháp
        fs::write(&test_file, "INVALID_TOML_CONTENT = {{ broken }}").unwrap();

        // Reload thất bại -> Cấu hình cũ được giữ nguyên an toàn
        let fail_res = cfg.reload_from(&test_file);
        assert!(fail_res.is_err());
        assert_eq!(cfg.poll_interval_ms, 1500);
        assert_eq!(cfg.backend, "ec");

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_load_or_default_fallback() {
        let temp_dir = std::env::temp_dir();
        let nonexistent_file = temp_dir.join("nonexistent_fan_ctl_test.toml");
        let _ = fs::remove_file(&nonexistent_file);

        let cfg = Config::load_or_default(&nonexistent_file);
        assert_eq!(cfg.poll_interval_ms, 1000);
        assert!(nonexistent_file.exists()); // Phải tự động tạo file mẫu

        let _ = fs::remove_file(nonexistent_file);
    }

    #[test]
    fn test_save_to_and_reload() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_save_config.toml");
        let _ = fs::remove_file(&test_file);

        let mut cfg = Config::default();
        cfg.poll_interval_ms = 750;
        cfg.cpu_curve = vec![
            CurvePoint::new(45, 30),
            CurvePoint::new(70, 75),
            CurvePoint::new(85, 100),
        ];

        let save_res = cfg.save_to(&test_file);
        assert!(save_res.is_ok());

        let loaded = Config::load_or_default(&test_file);
        assert_eq!(loaded.poll_interval_ms, 750);
        assert_eq!(loaded.cpu_curve.len(), 3);
        assert_eq!(loaded.cpu_curve[0].temp, 45);
        assert_eq!(loaded.cpu_curve[0].speed, 30);

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_get_config_path() {
        let path = get_config_path();
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }
}
