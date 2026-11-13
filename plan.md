MASTER IMPLEMENTATION PLAN: MODULE TELEMETRY BUFFER & NATIVE WIN32 GDI CHART (predator-fan-ctl)

Kế hoạch thi công dưới đây được thiết kế theo kiến trúc Zero-Allocation Ring Buffer kết hợp Win32 GDI Double-Buffering, bảo toàn tuyệt đối các tính năng hiện hữu của predator-fan-ctl (Smooth EPP Governor, Piecewise Linear Fan Curve, System Tray, Config Hot-Reload) và giữ mức tiêu thụ RAM tăng thêm dưới 64 KB, kích thước thực thi (.exe) không phình to do không sử dụng thêm bất kỳ crate đồ họa bên thứ ba nào.

1. Kiến trúc Tổng thể & Nguyên tắc Bảo toàn Hệ thống
Thành phần	Thiết kế kỹ thuật	Mục tiêu hiệu năng / Ràng buộc
Telemetry Storage	RingBuffer<TelemetrySample, N> cố định kích thước trong bộ nhớ	Chu kỳ lấy mẫu 2 giây 
→ 
1800 mẫu/giờ (
≈28.8 KB RAM). Nếu chu kỳ 1 giây 
→ 
3600 mẫu/giờ (
≈57.6 KB RAM).
Đồng bộ hóa (Sync)	Arc<RwLock<TelemetryHistory>> hoặc Arc<Mutex<TelemetryHistory>>	Luồng điều khiển quạt (Worker Thread) chỉ giữ lock 
<50 ns để push mẫu mới, không làm trễ chu kỳ ghi EC/WMI.
Giao diện Biểu đồ	Native Win32 Window (WS_OVERLAPPEDWINDOW) + GDI Memory DC	Chỉ khởi tạo GDI Objects khi cửa sổ mở; giải phóng hoàn toàn GDI handles khi ẩn/đóng cửa sổ để tránh rò rỉ bộ nhớ (GDI leak).
Tương tác (UX)	Double-buffered WM_PAINT, WM_TIMER (1000ms), WM_MOUSEMOVE	Vẽ mượt không nhấp nháy (flicker-free), hỗ trợ di chuột xem nhiệt độ chính xác tại từng mốc thời gian (Crosshair & Tooltip).
2. Phân rã Công việc Chi tiết (Step-by-Step Task Breakdown)
Giai đoạn 1: Khảo sát & Định vị Điểm Tích hợp (Pre-flight Audit)

Kỹ sư Antigravity thực hiện kiểm tra cấu trúc mã nguồn hiện tại trước khi sửa đổi:

Đọc Cargo.toml và các file trong src/ để xác định crate Win32 đang dùng (windows hay winapi) và danh sách features hiện có.

Kiểm tra Win32_Graphics_Gdi trong Cargo.toml. Đảm bảo các API GDI cơ bản đã được bật: CreateCompatibleDC, CreateCompatibleBitmap, SelectObject, DeleteObject, DeleteDC, BitBlt, CreatePen, CreateSolidBrush, CreateFontW, Polyline, FillRect, DrawTextW, SetBkMode, SetTextColor.

Xác định vị trí vòng lặp đọc nhiệt độ CPU/GPU (Control Loop) và nơi xử lý sự kiện WM_COMMAND của System Tray Menu.

Giai đoạn 2: Xây dựng Module Lưu trữ Lịch sử Nhiệt độ (src/telemetry.rs)

Tạo module độc lập quản lý bộ đệm vòng (Rolling Buffer) lưu trữ dữ liệu 1 giờ gần nhất mà không cấp phát động lại bộ nhớ trong quá trình chạy.

Định nghĩa cấu trúc mẫu dữ liệu (TelemetrySample):

Rust
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct TelemetrySample {
    pub timestamp_secs: u64, // Thời gian tương đối (monotonic) hoặc Unix epoch (giây)
    pub cpu_temp: u8,        // Nhiệt độ CPU (°C)
    pub gpu_temp: u8,        // Nhiệt độ GPU (°C)
    pub cpu_fan_pct: u8,     // % tốc độ quạt CPU (nếu có sẵn trong loop)
    pub gpu_fan_pct: u8,     // % tốc độ quạt GPU (nếu có sẵn trong loop)
}


(Kích thước mỗi bản ghi: 12–16 bytes).

Thiết kế TelemetryHistory (Fixed-Capacity Ring Buffer):

Định nghĩa hằng số dung lượng dựa trên chu kỳ poll_interval của ứng dụng:

Nếu vòng lặp quạt chạy mỗi 1 giây: pub const HISTORY_CAPACITY: usize = 3600;

Nếu vòng lặp quạt chạy mỗi 2 giây: pub const HISTORY_CAPACITY: usize = 1800;

Khuyến nghị: Chuẩn hóa bước ghi vào buffer là 1 mẫu / 1 giây (3600 mẫu) hoặc kiểm tra khoảng cách thời gian elapsed >= 1s trước khi push, đảm bảo đúng 3600 giây (1 giờ) bất kể tốc độ vòng lặp điều khiển quạt thay đổi khi hot-reload config.

Cấu trúc dữ liệu:

Rust
pub struct TelemetryHistory {
    buffer: Box<[TelemetrySample; 3600]>,
    head: usize,
    len: usize,
    start_instant: std::time::Instant,
}


Các phương thức bắt buộc:

pub fn new() -> Self: Khởi tạo mảng cố định trên Heap một lần duy nhất khi bật app.

pub fn push(&mut self, cpu_temp: u8, gpu_temp: u8, cpu_fan: u8, gpu_fan: u8): Ghi đè vào vị trí head, cập nhật head = (head + 1) % 3600, tăng len tối đa đến 3600.

pub fn iter_chronological(&self) -> impl Iterator<Item &TelemetrySample>: Trả về chuỗi dữ liệu theo đúng thứ tự thời gian từ cũ nhất đến mới nhất (tách 2 slice từ head..len và 0..head khi bộ đệm đã đầy).

pub fn stats(&self) -> Option<(u8, u8, f32, u8, u8, f32)>: Tính nhanh Min / Max / Avg của CPU và GPU trong 1 giờ qua để hiển thị trên Header của biểu đồ.

Giai đoạn 3: Tích hợp Thu thập Dữ liệu vào Luồng Điều khiển (Control Loop)

Tuyệt đối không làm thay đổi thuật toán Smooth EPP Governor hay Piecewise Linear Fan Curve hiện tại:

Khởi tạo let telemetry = Arc::new(Mutex::new(TelemetryHistory::new())); tại hàm main.

Truyền một bản Arc::clone(&telemetry) vào luồng điều khiển quạt (Worker Thread) và một bản vào luồng giao diện (UI / Tray Thread).

Ngay sau bước đọc nhiệt độ CPU/GPU và tính toán tốc độ quạt trong vòng lặp hiện hữu, chèn lệnh ghi không chặn (non-blocking / micro-lock):

Rust
if last_telemetry_record.elapsed() >= Duration::from_secs(1) {
    if let Ok(mut hist) = telemetry.try_lock() {
        hist.push(cpu_temp, gpu_temp, cpu_fan_pct, gpu_fan_pct);
        last_telemetry_record = Instant::now();
    }
}


Lưu ý: Sử dụng try_lock() hoặc giữ lock() chỉ trong thao tác gán mảng để đảm bảo luồng điều khiển phần cứng không bao giờ bị chặn bởi luồng vẽ UI.

Giai đoạn 4: Xây dựng Cửa sổ Biểu đồ Native Win32 GDI (src/chart_window.rs)

Thiết kế cửa sổ vẽ đồ thị hoàn toàn bằng Win32 GDI thuần, giao diện tối (Dark Mode) đồng bộ với phong cách Predator.

4.1. Quản lý Vòng đời Cửa sổ (Window Lifecycle)

Đăng ký Window Class PredatorTempChartWndClass với cờ CS_HREDRAW | CS_VREDRAW | CS_OWNDC.

Khi người dùng mở biểu đồ từ System Tray:

Nếu cửa sổ chưa tồn tại hoặc đang ẩn: Hiển thị cửa sổ (ShowWindow(hwnd, SW_SHOW)), đưa lên tiền cảnh (SetForegroundWindow(hwnd)), và bật bộ đếm thời gian SetTimer(hwnd, TIMER_ID_REFRESH, 1000, None) để tự động làm mới mỗi 1 giây.

Khi người dùng bấm nút Close (X) trên cửa sổ biểu đồ: Bắt sự kiện WM_CLOSE, hủy Timer (KillTimer), ẩn cửa sổ (ShowWindow(hwnd, SW_HIDE)) và trả về LRESULT(0) (tuyệt đối không gọi PostQuitMessage để tránh làm tắt toàn bộ ứng dụng System Tray chạy ngầm).

4.2. Kỹ thuật Vẽ Double-Buffering (WM_PAINT)

Để tránh hiện tượng nhấp nháy khi cập nhật mỗi giây hoặc khi kéo giãn cửa sổ (WM_SIZE):

Bắt WM_ERASEBKGND và trả về LRESULT(1) để ngăn Windows tự xóa nền trắng.

Trong WM_PAINT:

Gọi BeginPaint(hwnd, &mut ps) lấy hdc_screen.

Tạo Memory DC (CreateCompatibleDC(hdc_screen)) và Bitmap tương thích (CreateCompatibleBitmap(hdc_screen, width, height)).

Mọi thao tác vẽ thực hiện trên hdc_mem, sau đó chép toàn bộ ra màn hình bằng 1 lệnh BitBlt(..., SRCCOPY) duy nhất, rồi dọn dẹp (DeleteObject, DeleteDC, EndPaint).

4.3. Bố cục Đồ họa & Bảng màu (Predator Dark Theme)

Màu nền tổng thể: #121417 (RGB(18, 20, 23)).

Vùng vẽ đồ thị (Plot Area): Chừa lề Trái 50px (trục nhiệt độ °C), Dưới 35px (trục thời gian), Trên 55px (thanh tiêu đề & thông số tổng hợp), Phải 25px.

Lưới tọa độ (Gridlines):

Trục Y (Nhiệt độ): Cố định dải từ 30°C đến 100°C (bước chia mỗi 10°C: 30, 40, 50, 60, 70, 80, 90, 100). Vẽ đường ngang nét mảnh màu #282C34 (RGB(40, 44, 52)). Đường ngưỡng cảnh báo 85°C vẽ nét đứt màu đỏ sẫm #6E2626.

Trục X (Thời gian): Chia mốc mỗi 10 phút (-60m, -50m, -40m, -30m, -20m, -10m, Now).

Đường biểu diễn nhiệt độ (Data Series - Dùng Polyline):

CPU Temperature: Đường màu Cyan sáng #00E5FF (RGB(0, 229, 255)), độ dày 2px.

GPU Temperature: Đường màu Cam/Đỏ Predator #FF5252 hoặc #FF9100 (RGB(255, 145, 0)), độ dày 2px.

Công thức ánh xạ tọa độ điểm thứ 
𝑖
i:

Tọa độ ngang 
𝑋
X: Ánh xạ theo khoảng cách thời gian thực tế so với hiện tại hoặc theo chỉ số mẫu trong cửa sổ 3600 mẫu:

𝑋
=
plot_left
+
(
sample_index
3599.0
)
×
plot_width
X=plot_left+(
3599.0
sample_index
	​

)×plot_width

(Lưu ý: Nếu mới khởi động ứng dụng chưa đủ 1 giờ, có thể chọn căn phải về phía mốc Now hoặc giãn theo số mẫu hiện có, khuyến nghị căn phải về mốc Now để trục thời gian -60m .. Now luôn phản ánh đúng tỉ lệ thực tế, hoặc tự động co giãn trục X khi dữ liệu dưới 5 phút đầu).

Tọa độ dọc 
𝑌
Y:

𝑌
=
plot_bottom
−
(
clamp
(
𝑇
,
30
,
100
)
−
30
70.0
)
×
plot_height
Y=plot_bottom−(
70.0
clamp(T,30,100)−30
	​

)×plot_height

Thanh thông tin phía trên (Header Bar):

Hiển thị trực quan bằng font Segoe UI Semibold (10pt):

[■ CPU] Current: XX°C | Min: XX°C | Max: XX°C | Avg: XX.X°C

[■ GPU] Current: XX°C | Min: XX°C | Max: XX°C | Avg: XX.X°C

Tương tác Chuột (Interactive Crosshair & Tooltip):

Bắt sự kiện WM_MOUSEMOVE và WM_MOUSELEAVE.

Khi con trỏ chuột nằm trong vùng Plot Area, xác định mẫu dữ liệu gần nhất theo trục 
𝑋
X, vẽ một đường gióng dọc (Crosshair) và hộp Tooltip nhỏ hiển thị: Thời gian cách hiện tại (-XXm XXs) | CPU: XX°C | GPU: XX°C.

Giai đoạn 5: Tích hợp vào System Tray Menu & Hot-Reload Safe

Định nghĩa ID Menu mới:

Thêm hằng số IDM_SHOW_TEMP_CHART (ví dụ 1050, đảm bảo không trùng với các ID menu chuyển đổi chế độ quạt, EPP hay Reload Config hiện tại).

Cập nhật hàm tạo Context Menu của System Tray:

Chèn mục "Temperature History (1h Chart)..." vào vị trí hợp lý trên menu chuột phải (ngay phía trên mục Reload Config / Exit).

Tùy chọn nâng cao: Hỗ trợ mở nhanh biểu đồ khi người dùng Double-Click chuột trái (WM_LBUTTONDBLCLK) vào biểu tượng System Tray (nếu sự kiện này hiện chưa gán cho tính năng khác).

Xử lý sự kiện WM_COMMAND:

Khi nhận LOWORD(wparam) == IDM_SHOW_TEMP_CHART, gọi hàm chart_window::show_or_focus_chart_window().

Dọn dẹp khi Thoát ứng dụng (IDM_EXIT / WM_DESTROY):

Khi người dùng chọn thoát hoàn toàn ứng dụng từ Tray Menu, đảm bảo hủy cả cửa sổ biểu đồ (DestroyWindow) trước khi kết thúc message loop.

3. Kế hoạch Kiểm thử & Nghiệm thu (Verification & QA Checklist cho Antigravity)

Kỹ sư Antigravity phải thực hiện đầy đủ các bước kiểm thử sau và ghi kết quả vào báo cáo Markdown:

1
Kiểm tra Biên dịch & Kích thước Binary
Đảm bảo không phát sinh cảnh báo hoặc phình dung lượng

Chạy cargo check và cargo build --release. Xác nhận quá trình biên dịch đạt 0 errors, 0 warnings. Kiểm tra dung lượng file thực thi .exe trong target/release/ vẫn giữ dưới ngưỡng quy định (nhỏ hơn 3 MB).

2
Kiểm tra Tính toàn vẹn của Các tính năng Hiện hữu
Zero-Regression Test

Xác nhận Smooth EPP Governor, nội suy đường cong quạt (Piecewise Linear Fan Curve), tự động tải lại cấu hình (Config Hot-Reload), và các mục menu System Tray cũ hoạt động chính xác 100% như trước khi nâng cấp.

3
Kiểm tra Vòng đời Cửa sổ & Rò rỉ GDI (GDI Leak Test)
Quan trọng đối với phần mềm chạy ngầm

Mở cửa sổ biểu đồ từ System Tray, thay đổi kích thước cửa sổ (Resize/Maximize), đóng cửa sổ bằng nút X và mở lại liên tục 10 lần. Mở Task Manager (bật cột GDI Objects và Memory):

Xác nhận khi đóng cửa sổ biểu đồ, ứng dụng không bị tắt mà vẫn chạy dưới System Tray.

Xác nhận số lượng GDI Objects không tăng tịnh tiến sau mỗi chu kỳ vẽ WM_PAINT hoặc sau mỗi lần đóng/mở cửa sổ.

Xác nhận mức sử dụng RAM tổng thể nằm dưới ngưỡng 2.5 MB.

4
Kiểm tra Logic Bộ đệm Vòng (Ring Buffer Unit Test)
Xác minh giới hạn 1 giờ (3600 mẫu)

Viết và chạy unit test (cargo test) cho TelemetryHistory: nạp thử 5000 mẫu giả lập liên tiếp để xác nhận bộ đệm tự động ghi đè các mẫu cũ hơn 3600 giây, không panic, và iterator trả về đúng 3600 mẫu theo thứ tự thời gian tăng dần.

4. Yêu cầu Báo cáo Tiến trình dành cho Antigravity

Sau khi hoàn tất thi công và kiểm thử theo kế hoạch trên, Antigravity cần xuất báo cáo tiến trình qua file Markdown gửi lại cho Kiến trúc sư trưởng 4 Argon bao gồm:

Danh sách các file đã tạo mới và các file đã chỉnh sửa (kèm tóm tắt thay đổi).

Kết quả chạy cargo test cho module TelemetryHistory.

Thông số đo đạc thực tế sau khi build --release: Kích thước file .exe, mức tiêu thụ RAM và số lượng GDI Objects khi ẩn/hiện cửa sổ biểu đồ.

Các vấn đề phát sinh (nếu có) trong quá trình đồng bộ luồng giữa Worker Thread và Win32 UI Message Loop để nhận chỉ đạo tiếp theo.