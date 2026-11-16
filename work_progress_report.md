# BÁO CÁO TIẾN TRÌNH & HIỆN TRẠNG KỸ THUẬT: ĐO CÔNG SUẤT (W) VÀ XUNG NHỊP (MHZ) THEO THỜI GIAN

## 1. Yêu cầu Người Dùng (Feature Request)
- **Mục tiêu**: Thêm tính năng đo công suất (W) và xung nhịp (MHz/GHz) theo thời gian đo nhiệt độ, bổ sung vào thông tin biểu đồ (`chart_window.rs`) và giao diện phần mềm (`dashboard_window.rs`, `tray.rs`).
- **Phạm vi đối tượng**: Cả CPU (Intel Raptor Lake Core i7-13700HX) và GPU (NVIDIA GeForce RTX Laptop GPU).

## 2. Kết quả Khảo sát & Đo đạc Thực nghiệm (Proof of Concept Verification)
Chúng tôi đã viết script kiểm chứng trực tiếp trên phần cứng máy khách:
1. **GPU (NVIDIA NVML FFI - Zero-Allocation & < 5 micro-giây)**:
   - **GPU Power**: Gọi hàm `nvmlDeviceGetPowerUsage(device, &mut power_mw)`. Kết quả thực nghiệm: trả về chính xác công suất tiêu thụ GPU tính bằng milliwatts (ví dụ `19.19 W` / `19188 mW`), `status = NVML_SUCCESS (0)`.
   - **GPU Clock**: Gọi hàm `nvmlDeviceGetClockInfo(device, NVML_CLOCK_GRAPHICS, &mut clock_mhz)`. Kết quả thực nghiệm: trả về xung nhịp đồ họa tức thời (ví dụ `2250 MHz`), `status = NVML_SUCCESS (0)`.
   - **Cơ chế**: Tích hợp trực tiếp vào `NvmlGpuSensor` hiện có trong `src/sensor.rs`, không tốn thêm bộ nhớ, không phân bổ heap động.

2. **CPU (Windows Native PDH & PowrProf - Thời gian lấy mẫu ~0.5ms)**:
   - **CPU Package Power**: Sử dụng Windows Performance Data Helper (`pdh.dll`) với counter tiếng Anh `\Energy Meter(RAPL_Package0_PKG)\Power`. Kết quả đo thực nghiệm: `42.77 W` (chuyển đổi từ mW / 1000.0), `status = 0x0 (Success)`.
   - **CPU Clockspeed**: Có 2 kênh cực nhanh:
     - Kênh A: PDH Counter `\Processor Information(_Total)\Actual Frequency` -> kết quả thực nghiệm: `3707 MHz` (~3.71 GHz).
     - Kênh B: Win32 Native `CallNtPowerInformation(ProcessorInformation)` trong `powrprof.dll` -> trả về mảng `PROCESSOR_POWER_INFORMATION` gồm `CurrentMhz` của tất cả 20 core CPU với chi phí < 2 micro-giây, không cần I/O disk.
   - **Độ tin cậy & An toàn**: 100% sử dụng Win32 System API, không xung đột với Throttlestop, không cần can thiệp driver ring0 nguy hiểm, không gây lỗi BSOD.

3. **Cấu trúc Dữ liệu & Lưu trữ (Ring Buffer 1 Giờ - `src/telemetry.rs`)**:
   - Hiện tại: `TelemetrySample` gồm `timestamp_secs: u64`, `cpu_temp: u8`, `gpu_temp: u8`, `cpu_fan_pct: u8`, `gpu_fan_pct: u8` (16 bytes).
   - Nâng cấp: Mở rộng `TelemetrySample` lưu thêm:
     - `cpu_power_w: u16` (đơn vị: 0.1W hoặc deciwatts, 42.5W -> 425, hoặc lưu `f32` / `u16` W)
     - `gpu_power_w: u16` (đơn vị: 0.1W hoặc W)
     - `cpu_mhz: u16` (ví dụ: 800 - 5500 MHz)
     - `gpu_mhz: u16` (ví dụ: 210 - 2800 MHz)
   - Bộ đệm 3600 mẫu vẫn siêu gọn nhẹ: từ 57.6 KB tăng lên ~86.4 KB, hoàn toàn nằm gọn trên RAM, không rò rỉ.

4. **Biểu diễn trên Biểu đồ (`src/chart_window.rs`)**:
   - Thêm tab chọn hoặc toggle hiển thị:
     - Mode 1: Biểu đồ Nhiệt độ & Quạt (°C & %) - mặc định.
     - Mode 2: Biểu đồ Công suất (W) cho CPU & GPU (thang đo 0W - 150W).
     - Mode 3: Biểu đồ Xung nhịp (GHz) cho CPU & GPU (thang đo 0 - 5.5 GHz).
   - Hoặc hiển thị HUD / Infobar thời gian thực trên thanh trạng thái của biểu đồ: `CPU: 72°C | 42.5W | 3.8GHz` vs `GPU: 65°C | 65.0W | 2.1GHz`.

5. **Biểu diễn trên Dashboard UI (`src/dashboard_window.rs`)**:
   - Trên CPU Telemetry Card: Thêm dòng hiển thị `Công suất: 42.5 W` và `Xung nhịp: 3.82 GHz`.
   - Trên GPU Telemetry Card: Thêm dòng hiển thị `Công suất: 19.2 W` và `Xung nhịp: 2.25 GHz`.
   - Giữ nguyên bố cục co giãn linh hoạt (responsive layout) đã hoàn thiện ở v1.4.1.

## 3. Xin Chỉ Đạo từ Kiến Trúc Sư Trưởng
Kính nhờ Kiến trúc sư trưởng xem xét hiện trạng thực nghiệm trên, thẩm định phương án kiến trúc và vạch ra bản Kế hoạch tổng thể (`plan.md`) cùng phân rã tác vụ chi tiết để đội ngũ Antigravity triển khai các subagents thi công chuẩn xác nhất!
