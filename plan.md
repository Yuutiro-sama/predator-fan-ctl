<!-- Kế hoạch được lập bởi: Gemini 4 Hydrogen (Dự phòng - Suy luận cao) -->

1. Thẩm Định Kiến Trúc & Rà Soát Rủi Ro Kỹ Thuật (Architecture Review)

Kết quả khảo sát thực nghiệm (PoC) trên phần cứng mục tiêu (Intel Core i7-13700HX và NVIDIA GeForce RTX Laptop GPU) đạt độ chính xác cao và tuân thủ tốt triết lý Zero-Framework, Ultra-Lightweight Native Rust/Win32. Trước khi triển khai thi công diện rộng, Kiến trúc sư trưởng phê duyệt phương án và bổ sung 5 chốt chặn kỹ thuật bắt buộc để ngăn ngừa lỗi runtime:

Chốt chọn kênh đo xung nhịp CPU — Kênh B (CallNtPowerInformation) làm nguồn chính:

Sử dụng CallNtPowerInformation(ProcessorInformation, ...) từ powrprof.dll làm nguồn chính (độ trễ dưới 2 micro-giây, không phụ thuộc ngôn ngữ hệ điều hành), giữ PDH \Processor Information(_Total)\Actual Frequency làm fallback nếu syscall thất bại.

Quy tắc tổng hợp cho kiến trúc Hybrid (i7-13700HX — 6 P-cores + 8 E-cores = 20 logical threads): Không lấy trung bình cộng 20 luồng vì xung nhịp thấp của E-core sẽ kéo tụt chỉ số hiển thị khi hệ thống đang boost đơn nhân/game. Áp dụng max(CurrentMhz) trên toàn bộ mảng 20 phần tử PROCESSOR_POWER_INFORMATION để phản ánh chính xác xung nhịp boost đỉnh thực tế.

Chống lỗi bản địa hóa & rò rỉ handle trên Windows PDH (src/sensor.rs):

Bắt buộc dùng hàm PdhAddEnglishCounterW thay vì PdhAddCounterW cho counter \Energy Meter(RAPL_Package0_PKG)\Power để đảm bảo chạy ổn định trên mọi gói ngôn ngữ Windows.

Khởi tạo PDH_HQUERY và PDH_HCOUNTER một lần duy nhất khi khởi động service (CpuPowerSensor), gọi PdhCollectQueryData mồi 1 lần ở init(), sau đó chỉ gọi PdhCollectQueryData + PdhGetFormattedCounterValue mỗi chu kỳ 1 giây. Tuyệt đối không mở/đóng query theo từng tick.

Bảo vệ trạng thái ngủ NVIDIA Optimus D3cold (src/sensor.rs):

Trên laptop Acer Predator chạy chế độ hybrid GPU, khi dGPU chuyển sang trạng thái ngủ tiết kiệm điện (D3cold), hàm nvmlDeviceGetPowerUsage và nvmlDeviceGetClockInfo có thể trả về mã lỗi NVML_ERROR_NOT_SUPPORTED hoặc NVML_ERROR_GPU_IS_LOST.

Khi hàm NVML trả về status != 0, hệ thống phải gán êm (gpu_power_dw = 0, gpu_mhz = 0) mà không ghi log lỗi liên tục và không gọi lại các API đánh thức dGPU.

Tối ưu cấu trúc dữ liệu TelemetrySample bằng đơn vị Deciwatt (u16):

Lưu công suất bằng u16 đơn vị deciwatt (0.1 W) (ví dụ 42.77 W lưu thành 428, dải đo tới 6553.5 W) thay vì f32.

Kích thước struct đạt đúng 24 bytes (căn lề chuẩn 8-byte theo timestamp_secs: u64), giúp Ring Buffer 3.600 mẫu (1 giờ) chỉ chiếm 86.400 bytes (~84.4 KB) trên RAM.

Kỷ luật Zero-Allocation trong WM_PAINT (GDI):

Toàn bộ bút vẽ (HPEN), cọ tô (HBRUSH), font chữ (HFONT) cho các đường đồ thị mới và chuỗi định dạng UTF-16 ([u16; 64] trên stack) phải được cấp phát trước hoặc dùng stack buffer, giữ mức chiếm dụng RAM toàn ứng dụng dưới 6 MB và CPU dưới 0.3%.

2. Bản Thiết Kế Kiến Trúc Tổng Thể (plan.md)
Luồng Dữ Liệu & Kiến Trúc Mô-đun
Plaintext
[Hardware / OS Kernel]
 ├── NVML FFI (nvml.dll) ──────────► GpuMetrics { temp, power_dw, clock_mhz }
 ├── PowrProf (powrprof.dll) ──────► CpuClock   { max_mhz } (20 logical threads)
 └── PDH (pdh.dll - RAPL PKG) ─────► CpuPower   { package_dw }
           │
           ▼  (1 Hz Sampling Loop - Zero Heap Alloc)
[src/telemetry.rs]
 └── TelemetrySample (24 bytes) ───► RingBuffer<TelemetrySample, 3600> (~84.4 KB)
           │
           ├──► [src/chart_window.rs]     (3 Modes: Temp/Fan | Power W | Clock GHz + Live HUD)
           ├──► [src/dashboard_window.rs] (CPU & GPU Cards: + Công suất W, + Xung nhịp GHz)
           └──► [src/tray.rs]             (Compact Tooltip: °C | W | GHz)

Đặc Tả Cấu Trúc Dữ Liệu Chuẩn (src/telemetry.rs)
Rust
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TelemetrySample {
    pub timestamp_secs: u64, // 8 bytes
    pub cpu_temp: u8,        // 1 byte  (°C)
    pub gpu_temp: u8,        // 1 byte  (°C)
    pub cpu_fan_pct: u8,     // 1 byte  (%)
    pub gpu_fan_pct: u8,     // 1 byte  (%)
    pub cpu_power_dw: u16,   // 2 bytes (Deciwatts: 425 = 42.5 W)
    pub gpu_power_dw: u16,   // 2 bytes (Deciwatts: 192 = 19.2 W)
    pub cpu_mhz: u16,        // 2 bytes (MHz: 3820 = 3.82 GHz)
    pub gpu_mhz: u16,        // 2 bytes (MHz: 2250 = 2.25 GHz)
} // Total: 20 bytes payload + 4 bytes alignment padding = 24 bytes

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartMode {
    TempAndFan = 0, // Nhiệt độ (°C) & Tốc độ quạt (%) - Mặc định
    Power = 1,      // Công suất tiêu thụ CPU & GPU (W)
    Clock = 2,      // Xung nhịp CPU & GPU (GHz)
}

3. Phân Rã Tác Vụ Chi Tiết Cho Đội Ngũ Subagents (Step-by-Step Breakdown)

Antigravity triển khai tuần tự/song song 4 nhóm tác vụ sau đây và cập nhật trạng thái vào file tiến trình:

Phase 1: Tầng Cảm Biến Phần Cứng (Subagent-Sensor — src/sensor.rs)

Task 1.1 — Mở rộng NvmlGpuSensor:

Khai báo FFI signature cho nvmlDeviceGetPowerUsage(device: nvmlDevice_t, power: *mut u32) -> nvmlReturn_t và nvmlDeviceGetClockInfo(device: nvmlDevice_t, clock_type: u32, clock: *mut u32) -> nvmlReturn_t (với NVML_CLOCK_GRAPHICS = 0).

Viết helper read_power_and_clock(&self) -> (u16, u16): quy đổi power_mw sang deciwatt ((power_mw + 50) / 100) as u16, trả về (0, 0) an toàn nếu NVML báo lỗi hoặc GPU đang ngủ D3cold.

Task 1.2 — Xây dựng CpuClockSensor (PowrProf Win32 FFI):

Định nghĩa struct PROCESSOR_POWER_INFORMATION (Number, MaxMhz, CurrentMhz, MhzLimit, MaxIdleState, CurrentIdleState — đều kiểu u32).

Dùng GetSystemInfo lấy chính xác số logical processor (dwNumberOfProcessors, ví dụ 20 trên i7-13700HX), cấp phát buffer cố định Vec<PROCESSOR_POWER_INFORMATION> một lần duy nhất trong new().

Hàm sample_max_mhz(&mut self) -> u16: gọi CallNtPowerInformation(11 /* ProcessorInformation */, ...) và trả về giá trị CurrentMhz lớn nhất trong mảng.

Task 1.3 — Xây dựng CpuPowerSensor (PDH English Counter):

Khởi tạo PdhOpenQueryW, thêm counter \Energy Meter(RAPL_Package0_PKG)\Power qua PdhAddEnglishCounterW, thực hiện PdhCollectQueryData lần đầu để lấy mốc baseline.

Hàm sample_deciwatts(&mut self) -> u16: gọi PdhCollectQueryData, đọc PdhGetFormattedCounterValue với cờ PDH_FMT_DOUBLE, quy đổi mW sang deciwatt ((val_mw / 100.0).round() as u16). Triển khai Drop gọi PdhCloseQuery.

Phase 2: Tầng Lưu Trữ & Vòng Lặp Lấy Mẫu (Subagent-Telemetry — src/telemetry.rs, src/main.rs)

Task 2.1 — Nâng cấp TelemetrySample và TelemetryHistory:

Cập nhật struct TelemetrySample lên 24 bytes kèm các method tiện ích (cpu_power_w(&self) -> f32, gpu_power_w(&self) -> f32, cpu_ghz(&self) -> f32, gpu_ghz(&self) -> f32).

Kiểm tra dung lượng RingBuffer (3.600 phần tử) đảm bảo không tái cấp phát bộ nhớ sau khởi tạo.

Task 2.2 — Đồng bộ hóa luồng thu thập 1 Hz:

Tích hợp CpuClockSensor, CpuPowerSensor, và NvmlGpuSensor vào vòng lặp thu thập định kỳ mỗi 1.000 ms, đẩy mẫu mới vào TelemetryHistory và kích hoạt InvalidateRect cho cửa sổ Dashboard/Chart nếu đang mở.

Phase 3: Biểu Đồ GDI Đa Chế Độ & Live HUD (Subagent-UI-Chart — src/chart_window.rs)

Task 3.1 — Thanh chọn chế độ (Mode Tabs) & Xử lý chuột/phím:

Lưu trạng thái active_mode: ChartMode trong state của ChartWindow.

Vẽ 3 tab gọn trên thanh tiêu đề phụ của biểu đồ: [Nhiệt độ & Quạt], [Công suất (W)], [Xung nhịp (GHz)].

Xử lý sự kiện WM_LBUTTONDOWN (hit-test vào vùng 3 tab) và WM_KEYDOWN (phím 1, 2, 3 để chuyển nhanh chế độ).

Task 3.2 — Render đồ thị Công suất (ChartMode::Power) & Xung nhịp (ChartMode::Clock):

Chế độ Công suất (W): Trục Y thang 0 – 150 W (tự động mở rộng lên 200 W nếu đỉnh vượt 150 W), chia vạch mỗi 30 W. Vẽ 2 đường Polyline: CPU Power (xanh dương/cyan đặc trưng) và GPU Power (xanh lá/cam đặc trưng).

Chế độ Xung nhịp (GHz): Trục Y thang 0.0 – 5.5 GHz, chia vạch mỗi 1.0 GHz. Vẽ 2 đường Polyline: CPU Clock (cpu_mhz / 1000.0) và GPU Clock (gpu_mhz / 1000.0).

Task 3.3 — Thanh trạng thái HUD thời gian thực (Live Infobar):

Hiển thị cố định thông số tức thời của mẫu mới nhất ngay dưới thanh tab:CPU: 72°C | 42.8W | 3.82 GHz   •   GPU: 65°C | 19.2W | 2.25 GHz

Phase 4: Nâng Cấp Dashboard & System Tray (Subagent-UI-Dash — src/dashboard_window.rs, src/tray.rs)

Task 4.1 — Bổ sung thông số lên CPU & GPU Telemetry Card (src/dashboard_window.rs):

Trong hàm tính toán layout đáp ứng (WM_SIZE / DPI scaling từ v1.4.1), bổ sung 2 hàng chỉ số phụ trên mỗi Card:

CPU Card: Công suất: XX.X W | Xung nhịp: X.XX GHz

GPU Card: Công suất: XX.X W | Xung nhịp: X.XX GHz

Đảm bảo không tràn chữ khi thu nhỏ cửa sổ về kích thước tối thiểu.

Task 4.2 — Cập nhật Tooltip khay hệ thống (src/tray.rs):

Định dạng chuỗi szTip (tối đa 127 ký tự UTF-16 + null terminator) hiển thị đầy đủ nhiệt độ, công suất và xung nhịp khi di chuột vào icon tray.

Phase 5: Kiểm Thử Tích Hợp & Nghiệm Thu (Subagent-QA)

Task 5.1 — Kiểm thử biên dịch & cảnh báo: Chạy cargo clippy --all-targets và cargo build --release (0 warnings, 0 errors).

Task 5.2 — Kiểm thử thực tế 3 kịch bản:

Idle / Optimus Sleep: Kiểm tra khi dGPU tắt (0W / 0 MHz), ứng dụng không báo lỗi và không đánh thức GPU.

Heavy Load (CPU + GPU): Chạy tải nặng đồng thời, xác nhận biểu đồ Công suất vượt ngưỡng 100W+ tự động co giãn trục Y mượt mà, xung nhịp CPU P-core hiển thị chính xác mức boost (~4.5–5.0 GHz).

Kiểm tra rò rỉ tài nguyên: Theo dõi Working Set RAM và số lượng GDI Objects trong Task Manager sau 15 phút mở liên tục cửa sổ biểu đồ (yêu cầu GDI Objects đứng yên không tăng dần).

4. Quy Chuẩn Báo Cáo Hoàn Thành Từ Antigravity

Sau khi các subagents hoàn tất thi công, Antigravity gửi lại báo cáo Markdown cho Kiến trúc sư trưởng kèm các bằng chứng xác thực:

Kết quả cargo build --release và kích thước file thực thi .exe.

Bảng đối chiếu thông số đo được giữa predator-fan-ctl và HWiNFO64 tại 2 trạng thái (Idle và Full Load).

Chỉ số tài nguyên thực tế (Private Working Set RAM, CPU %, GDI Objects count) khi mở đồng thời Dashboard và Chart Window.