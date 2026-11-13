//! Module Quản lý Windows Service qua Win32 Service Control Manager (SCM).
//!
//! Đảm nhiệm việc phát hiện, tạm dừng an toàn dịch vụ OEM (mặc định là `PredatorSenseService` hoặc `AcerSysMonitorService`)
//! khi ứng dụng chạy để tránh xung đột ghi đè thanh ghi quạt ACPI/EC, và tự động khôi phục
//! dịch vụ về trạng thái ban đầu khi ứng dụng kết thúc thông qua cơ chế RAII (`ServiceGuard`).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::thread::sleep;
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::System::Services::*;

/// Trạng thái của một Windows Service cụ thể được quản lý.
#[derive(Debug, Clone)]
pub struct SingleServiceState {
    pub name: String,
    pub wide_name: Vec<u16>,
    pub was_running: bool,
}

/// Bộ canh gác RAII cho các Windows Service OEM của Acer.
///
/// Quản lý danh sách các service OEM tiềm năng (ví dụ `"AASSvc"`, `"ASMSvc"`, `"PredatorSenseService"`).
/// Khi được khởi tạo (`acquire_many`), nếu dịch vụ nào đang chạy và cờ `manage` được bật,
/// guard sẽ ra lệnh dừng dịch vụ và ghi nhớ.
/// Khi guard bị hủy (`Drop`), tất cả dịch vụ trước đó đang chạy sẽ tự động được khởi động lại.
#[derive(Debug)]
pub struct ServiceGuard {
    services: Vec<SingleServiceState>,
    manage: bool,
}

impl ServiceGuard {
    /// Khởi tạo và yêu cầu quyền kiểm soát một danh sách dịch vụ OEM tiềm năng.
    pub fn acquire_many(names: &[&str], manage: bool) -> Self {
        let mut services = Vec::new();
        for &name in names {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            let wide_name: Vec<u16> = OsStr::new(trimmed).encode_wide().chain(Some(0)).collect();
            services.push(SingleServiceState {
                name: trimmed.to_string(),
                wide_name,
                was_running: false,
            });
        }

        let mut guard = Self { services, manage };

        if manage {
            guard.stop_all_running();
        }

        guard
    }

    /// Khởi tạo và yêu cầu quyền kiểm soát dịch vụ theo tên đơn lẻ (hỗ trợ tương thích ngược).
    pub fn acquire(name: &str, manage: bool) -> Self {
        Self::acquire_many(&[name], manage)
    }

    /// Tên đại diện của các dịch vụ đang được quản lý.
    pub fn name(&self) -> String {
        self.services
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Cho biết có bất kỳ dịch vụ nào đang chạy trước khi guard can thiệp hay không.
    #[inline]
    pub fn was_running(&self) -> bool {
        self.services.iter().any(|s| s.was_running)
    }

    /// Danh sách tên các dịch vụ thực tế đã được tạm dừng.
    pub fn stopped_services(&self) -> Vec<&str> {
        self.services
            .iter()
            .filter(|s| s.was_running)
            .map(|s| s.name.as_str())
            .collect()
    }

    /// Cho biết guard có đang ở chế độ chủ động quản lý service hay không.
    #[inline]
    pub fn is_managed(&self) -> bool {
        self.manage
    }

    /// Dừng tất cả dịch vụ trong danh sách nếu đang ở trạng thái RUNNING hoặc START_PENDING.
    pub fn stop_all_running(&mut self) {
        unsafe {
            let Ok(scm) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) else {
                return;
            };

            for service in &mut self.services {
                let Ok(svc) = OpenServiceW(
                    scm,
                    PCWSTR(service.wide_name.as_ptr()),
                    SERVICE_QUERY_STATUS | SERVICE_STOP | SERVICE_START,
                ) else {
                    continue;
                };

                let mut status = SERVICE_STATUS::default();
                if QueryServiceStatus(svc, &mut status).is_ok() {
                    if status.dwCurrentState == SERVICE_RUNNING
                        || status.dwCurrentState == SERVICE_START_PENDING
                    {
                        service.was_running = true;
                        let _ = ControlService(svc, SERVICE_CONTROL_STOP, &mut status);

                        // Chờ tối đa 3 giây để dịch vụ dừng hẳn
                        for _ in 0..30 {
                            sleep(Duration::from_millis(100));
                            if QueryServiceStatus(svc, &mut status).is_ok() {
                                if status.dwCurrentState == SERVICE_STOPPED {
                                    break;
                                }
                            }
                        }
                    }
                }

                let _ = CloseServiceHandle(svc);
            }

            let _ = CloseServiceHandle(scm);
        }
    }

    /// Khôi phục trạng thái ban đầu của tất cả các dịch vụ đã bị tạm dừng.
    pub fn restore(&self) {
        if !self.manage {
            return;
        }

        unsafe {
            let Ok(scm) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) else {
                return;
            };

            for service in &self.services {
                if !service.was_running {
                    continue;
                }

                if let Ok(svc) = OpenServiceW(
                    scm,
                    PCWSTR(service.wide_name.as_ptr()),
                    SERVICE_START | SERVICE_QUERY_STATUS,
                ) {
                    let _ = StartServiceW(svc, None);

                    let mut status = SERVICE_STATUS::default();
                    for _ in 0..15 {
                        sleep(Duration::from_millis(100));
                        if QueryServiceStatus(svc, &mut status).is_ok() {
                            if status.dwCurrentState == SERVICE_RUNNING {
                                break;
                            }
                        }
                    }

                    let _ = CloseServiceHandle(svc);
                }
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

/// Kiểm tra xem một Windows Service có đang chạy hay không.
pub fn is_service_running(name: &str) -> bool {
    let wide_name: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
    unsafe {
        let Ok(scm) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) else {
            return false;
        };

        let Ok(svc) = OpenServiceW(
            scm,
            PCWSTR(wide_name.as_ptr()),
            SERVICE_QUERY_STATUS,
        ) else {
            let _ = CloseServiceHandle(scm);
            return false;
        };

        let mut status = SERVICE_STATUS::default();
        let running = if QueryServiceStatus(svc, &mut status).is_ok() {
            status.dwCurrentState == SERVICE_RUNNING
        } else {
            false
        };

        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
        running
    }
}

/// Truy vấn trạng thái hiện tại của Windows Service theo mã dwCurrentState.
pub fn query_service_state(name: &str) -> Option<u32> {
    let wide_name: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
    unsafe {
        let scm = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT).ok()?;
        let svc = OpenServiceW(
            scm,
            PCWSTR(wide_name.as_ptr()),
            SERVICE_QUERY_STATUS,
        ).ok();

        let state = match svc {
            Some(s) => {
                let mut status = SERVICE_STATUS::default();
                let res = if QueryServiceStatus(s, &mut status).is_ok() {
                    Some(status.dwCurrentState.0)
                } else {
                    None
                };
                let _ = CloseServiceHandle(s);
                res
            }
            None => None,
        };

        let _ = CloseServiceHandle(scm);
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_guard_unmanaged() {
        let guard = ServiceGuard::acquire("NonExistentTestService12345", false);
        assert_eq!(guard.name(), "NonExistentTestService12345");
        assert!(!guard.was_running());
        assert!(!guard.is_managed());
        // Drop should be a no-op and never panic
    }

    #[test]
    fn test_query_nonexistent_service() {
        let state = query_service_state("NonExistentTestService99999");
        assert!(state.is_none());
    }

    #[test]
    fn test_is_running_nonexistent_service() {
        let running = is_service_running("NonExistentTestService99999");
        assert!(!running);
    }
}
