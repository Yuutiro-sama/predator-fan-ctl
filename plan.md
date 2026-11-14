Bản Master Implementation Plan dưới đây định hình toàn bộ kiến trúc kỹ thuật và quy trình thi công chi tiết cho phân hệ Dashboard / Main UI Window của dự án predator-fan-ctl trên nền tảng Rust Win32 GDI thuần.

1. TỔNG QUAN KIẾN TRÚC & BẢNG MÀU (DESIGN SYSTEM)

Để đảm bảo mức tiêu thụ RAM dưới 15MB (thực tế mục tiêu giữ quanh mức 3MB - 5MB khi mở cửa sổ) và 0% CPU khi nhàn rỗi, cửa sổ Dashboard sử dụng kiến trúc Event-Driven Off-Screen Double-Buffered Win32 GDI kết hợp với cơ chế GDI Object Cache (khởi tạo Font/Pen/Brush một lần hoặc theo chu kỳ vẽ có kiểm soát, giải phóng chặt chẽ).

1.1. Hệ thống Bảng màu (Color Palette & Theme System)

Hệ thống hỗ trợ chuyển đổi linh hoạt giữa Predator Dark Theme (mặc định, đồng bộ hoàn toàn với cửa sổ Biểu đồ 1h) và Clean Light Theme thông qua cấu trúc ThemePalette. Lưu ý: Win32 COLORREF sử dụng định dạng 0x00BBGGRR.

Thành phần Giao diện	Mã Hex (Dark Predator)	Macro RGB(R, G, B) (Dark)	Mã Hex (Light Mode)	Vai trò
Background chính	#121417	RGB(18, 20, 23)	#F4F6F9	Nền tổng thể cửa sổ Dashboard
Card / Panel Surface	#16191E	RGB(22, 25, 30)	#FFFFFF	Nền Card CPU, GPU và khu vực điều khiển
Card / Button Border	#343B48	RGB(52, 59, 72)	#D0D7DE	Viền phân tách Card và nút trạng thái bình thường
Button Normal Surface	#1E2229	RGB(30, 34, 41)	#EAECEF	Nền nút bấm trạng thái nghỉ
Button Hover Surface	#282E38	RGB(40, 46, 56)	#DCE1E7	Nền nút khi rê chuột (Hover - sáng hơn)
Button Pressed Surface	#0F1114	RGB(15, 17, 20)	#C8D0D8	Nền nút khi nhấn giữ chuột (Tactile Sink)
CPU Accent (Cyan)	#00E5FF	RGB(0, 229, 255)	#0097A7	Thanh tiến trình, số liệu nhiệt độ/quạt CPU
GPU Accent (Orange)	#FF9100	RGB(255, 145, 0)	#E65100	Thanh tiến trình, số liệu nhiệt độ/quạt GPU
Text Primary	#ECEFF4	RGB(236, 239, 244)	#1F2328	Chữ tiêu đề, thông số chính
Text Secondary	#8B949E	RGB(139, 148, 158)	#57606A	Nhãn phụ (RPM, Duty %, chú thích)
Danger / Exit Hover	#FF3B30	RGB(255, 59, 48)	#D32F2F	Viền/Nền cảnh báo cho nút "Thoát" hoặc Quá nhiệt
1.2. Bố cục Khung hình (Viewport Layout - Kích thước chuẩn 540 x 440 px)

Cửa sổ sử dụng style WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX (cố định kích thước để tránh vỡ layout GDI và giảm thiểu cấp phát lại Bitmap đệm).

Header Bar (Y: 0..48): Tiêu đề "PREDATOR FAN CONTROL // DASHBOARD", chỉ báo trạng thái Daemon (ACTIVE), và nút chuyển nhanh Theme (Dark/Light).

Khu vực Telemetry Cards (Y: 56..200): Chia 2 cột cân xứng:

CPU Card (X: 16..262, Y: 56..200): Hiển thị Nhiệt độ lớn (32pt Bold), thanh Bar % tốc độ quạt, thông số % quạt và RPM thực tế.

GPU Card (X: 278..524, Y: 56..200): Hiển thị tương tự với tông màu Cam #FF9100.

Khu vực Chế độ Quạt - Profile Selector (Y: 214..274): 3 nút Segmented Pill nằm ngang (Active Curve, Turbo, Silent). Nút đang kích hoạt sẽ sáng viền Cyan #00E5FF và tô nền nhấn.

Khu vực Tối ưu Nhiệt - CPU EPP Governor (Y: 286..334): Thanh ngang chứa nhãn trạng thái "Khóa nhiệt độ CPU (Smooth EPP Governor)" và nút gạt/bấm Toggle ON/OFF trực quan.

Khu vực Công cụ & Hành động (Y: 348..424): Lưới 2x2 gồm 4 nút chức năng bo góc:

Chỉnh Fan Curve (Mở file config.toml hoặc trình chỉnh sửa).

Xem Biểu đồ 1h (Gọi chart_window::show_or_focus()).

Tải lại cấu hình (Kích hoạt hot-reload ngay lập tức và nháy thông báo xác nhận).

Thoát ứng dụng (Viền đỏ cảnh báo khi hover, đóng sạch daemon và trả quyền EC về BIOS).

2. KẾ HOẠCH THI CÔNG CHI TIẾT TỪNG BƯỚC (STEP-BY-STEP TASK BREAKDOWN)

Kỹ sư Antigravity thực hiện tuần tự 5 Phase dưới đây.

PHASE 1: Tái cấu trúc Khay hệ thống (System Tray Left-Click Routing)

Mục tiêu: Giữ nguyên Context Menu khi click chuột phải (WM_RBUTTONUP), bổ sung bắt sự kiện click chuột trái (WM_LBUTTONUP) để mở hoặc đưa cửa sổ Dashboard lên tiền cảnh.

Mở file xử lý Tray / Main Loop (ví dụ: src/tray.rs hoặc src/main.rs).

Cập nhật hằng số thông điệp Callback của Tray Icon (WM_TRAYICON):
Trong hàm wndproc của cửa sổ ẩn quản lý Tray:

Rust
WM_TRAYICON => {
    let event = (lparam.0 & 0xFFFF) as u32;
    match event {
        WM_LBUTTONUP => {
            // Mở hoặc Focus cửa sổ Dashboard chính
            crate::dashboard_window::show_or_focus_dashboard();
        }
        WM_RBUTTONUP | WM_CONTEXTMENU => {
            // Giữ nguyên logic hiển thị Popup Menu chuột phải hiện tại
            show_tray_context_menu(hwnd);
        }
        _ => {}
    }
    LRESULT(0)
}


Bổ sung mục Menu dự phòng: Thêm dòng "Mở Bảng điều khiển (Dashboard)" ở vị trí đầu tiên (in đậm bằng SetMenuDefaultItem) trong menu chuột phải để người dùng có 2 cách truy cập đồng nhất.

PHASE 2: Xây dựng Hệ thống Nút bấm Phản hồi Xúc giác (Tactile UI Engine)

Mục tiêu: Tạo trải nghiệm nút bấm bo góc (RoundRect), sáng lên khi rê chuột (Hover), và có độ lún vật lý (Pressed - dịch chuyển nội dung/khung 1-2px xuống dưới) mà không tốn CPU.

Khởi tạo module src/dashboard_window.rs.

Định nghĩa cấu trúc Nút và Trạng thái Tương tác:

Rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonId {
    ProfileCurve,
    ProfileTurbo,
    ProfileSilent,
    ToggleEpp,
    OpenCurveEditor,
    OpenTelemetryChart,
    ReloadConfig,
    ExitApp,
    ToggleTheme,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UiState {
    Normal,
    Hovered,
    Pressed,
}

pub struct TactileButton {
    pub id: ButtonId,
    pub rect: RECT,
    pub label: &'static str,
    pub state: UiState,
    pub is_active: bool, // Dùng cho Profile đang chọn hoặc Toggle EPP đang bật
    pub is_danger: bool, // Dùng cho nút Thoát
}


Thuật toán Vẽ Nút Tactile (draw_tactile_button):

Hiệu ứng độ nảy (Tactile Bounce/Sink):

Khi state == UiState::Hovered: Giữ nguyên rect hoặc nâng nhẹ biên trên -1px, đổi màu nền sang Button Hover Surface, đổi màu viền sang CPU Accent (#00E5FF) (hoặc #FF3B30 nếu is_danger == true).

Khi state == UiState::Pressed: Dịch chuyển khung vẽ và tọa độ chữ xuống dưới và sang phải +2px (rect.top += 2; rect.bottom += 2; rect.left += 1; rect.right += 1;), tạo cảm giác phím cơ lún xuống khi bấm chuột.

Khi is_active == true (ví dụ Profile đang được chọn hoặc EPP đang bật): Tô nền pha màu Cyan đậm RGB(0, 65, 80) và viền Cyan sáng RGB(0, 229, 255) dày 2px.

Dùng hàm Win32 RoundRect(hdc, left, top, right, bottom, 10, 10) để tạo góc bo mượt 10px.

Cơ chế TrackMouseEvent chống kẹt trạng thái Hover:

Khi xử lý WM_MOUSEMOVE: Kiểm tra tọa độ (x, y) bằng PtInRect.Chỉ gọi InvalidateRect(hwnd, None, FALSE) khi và chỉ khi hovered_button thay đổi so với frame trước (đảm bảo rê chuột trong cùng 1 vùng không vẽ lại liên tục, giữ CPU ~0%).

Đăng ký TrackMouseEvent với cờ TME_LEAVE. Khi nhận WM_MOUSELEAVE, đặt toàn bộ trạng thái nút về Normal và vẽ lại 1 lần duy nhất.

PHASE 3: Thiết kế Luồng Dữ liệu Thời gian thực (Zero-Contention State Snapshot)

Mục tiêu: Đọc dữ liệu từ vòng lặp điều khiển quạt sang giao diện Dashboard mà không gây khóa (lock contention) làm trễ chu kỳ ghi thanh ghi EC.

Cấu trúc DashboardSnapshot:
Bổ sung hoặc mở rộng cấu trúc trạng thái chia sẻ (nằm trong Arc<Mutex<SharedAppState>> hoặc sử dụng AtomicU32 đóng gói):

Rust
#[derive(Clone, Copy, Default)]
pub struct DashboardSnapshot {
    pub cpu_temp_c: u32,
    pub cpu_fan_pct: u32,
    pub cpu_fan_rpm: u32,
    pub gpu_temp_c: u32,
    pub gpu_fan_pct: u32,
    pub gpu_fan_rpm: u32,
    pub active_profile: FanProfileMode, // Curve, Turbo, Silent
    pub epp_locked: bool,
    pub reload_flash_ticks: u8, // Đếm ngược hiệu ứng báo "Đã tải lại cấu hình!"
}


Cách tính RPM (nếu WMI/EC trả về RPM hoặc nội suy):

Nếu phần cứng trả trực tiếp thanh ghi RPM từ EC: đọc thẳng giá trị thực.

Nếu phần cứng trên model hiện tại chỉ trả về % quạt: đọc từ thanh ghi tach hoặc nội suy chuẩn xác theo dải tốc độ tối đa của quạt Predator (tối đa ~5000 - 5600 RPM ở 100%) kèm ký hiệu rõ ràng.

PHASE 4: Kỹ thuật Vẽ GDI Double-Buffering & Quản lý Vòng đời Cửa sổ (Singleton)

Mục tiêu: Chống nhấp nháy (zero-flicker) tuyệt đối, đóng [X] chỉ ẩn cửa sổ và ngắt bộ đếm thời gian để CPU về 0%.

Quản lý Singleton Window Handle:

Lưu DASHBOARD_HWND: AtomicIsize = AtomicIsize::new(0).

Trong hàm show_or_focus_dashboard():

Nếu DASHBOARD_HWND != 0: Gọi ShowWindow(hwnd, SW_RESTORE), SetForegroundWindow(hwnd), khởi động lại Timer cập nhật giao diện SetTimer(hwnd, IDT_DASHBOARD_REFRESH, 1000, None), và gọi InvalidateRect để vẽ ngay lập tức.

Nếu DASHBOARD_HWND == 0: Đăng ký WNDCLASSEXW với hbrBackground = None (chống chớp trắng), tạo cửa sổ và lưu HWND.

Tối ưu CPU 0% khi Cửa sổ Ẩn hoặc Minimize:

Bắt sự kiện đóng cửa sổ (WM_CLOSE):TUYỆT ĐỐI KHÔNG gọi DestroyWindow hay PostQuitMessage. Thay vào đó:

Rust
WM_CLOSE => {
    unsafe {
        let _ = KillTimer(hwnd, IDT_DASHBOARD_REFRESH);
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    LRESULT(0)
}


Bắt sự kiện thu nhỏ (WM_SIZE):
Nếu wparam.0 as u32 == SIZE_MINIMIZED, gọi KillTimer(hwnd, IDT_DASHBOARD_REFRESH). Khi SIZE_RESTORED, bật lại SetTimer chu kỳ 1000ms (1 giây/lần). Nhờ đó, khi không nhìn vào bảng điều khiển, tiến trình giao diện tốn 0.00% CPU.

Quy trình Vẽ WM_PAINT với Double-Buffering chuẩn:

Bắt WM_ERASEBKGND -> Trả về LRESULT(1) ngay lập tức.

Trong WM_PAINT:

BeginPaint(hwnd, &mut ps)

Tạo mem_dc = CreateCompatibleDC(hdc) và mem_bmp = CreateCompatibleBitmap(hdc, width, height).

Chọn old_bmp = SelectObject(mem_dc, mem_bmp).

SetBkMode(mem_dc, TRANSPARENT).

Vẽ nền & Header: Tô nền #121417, vẽ đường kẻ phân tách #343B48.

Vẽ Card CPU (#00E5FF) & Card GPU (#FF9100):

Dùng RoundRect vẽ khung Card nền #16191E, viền #343B48.

Vẽ nhãn "CPU TELEMETRY" / "GPU TELEMETRY".

Vẽ số nhiệt độ cỡ lớn (CreateFontW cao -36, FW_BOLD, "Segoe UI"): Ví dụ "64°C".

Vẽ thanh Progress Bar ngang biểu diễn % quạt: Khung nền tối #0F1114, phần trăm thực tế tô bằng Brush màu Cyan (#00E5FF) cho CPU và Orange (#FF9100) cho GPU.

Vẽ dòng thông số dưới cùng của Card: "Tốc độ quạt: 55%  |  3100 RPM".

Vẽ Hàng Nút Chọn Profile & Toggle EPP: Duyệt mảng TactileButton và gọi hàm draw_tactile_button.

Vẽ Lưới 4 Nút Chức năng Cuối: Hiển thị rõ trạng thái hover/click.

Blit ra màn hình: BitBlt(hdc, 0, 0, width, height, mem_dc, 0, 0, SRCCOPY).

Dọn dẹp GDI (Chống rò rỉ RAM tuyệt đối): Khôi phục SelectObject(mem_dc, old_bmp), gọi DeleteObject(mem_bmp), DeleteDC(mem_dc) và EndPaint(hwnd, &ps). Mọi HBRUSH, HPEN, HFONT tạo tạm trong hàm vẽ đều phải được DeleteObject trước khi thoát hàm (hoặc lưu tĩnh trong struct GdiResources khởi tạo 1 lần).

PHASE 5: Liên kết Hành động Nút bấm (Action Dispatchers)

Khi người dùng nhấn (WM_LBUTTONDOWN) và nhả chuột (WM_LBUTTONUP) trên cùng một ButtonId, thực thi lệnh tương ứng:

ButtonId	Hành động Thực thi (Action Handler)
ProfileCurve	Chuyển chế độ sang Active Curve (điều khiển tự động theo đồ thị nhiệt), ghi nhận vào State và kích hoạt chu kỳ tính toán quạt ngay lập tức.
ProfileTurbo	Chuyển chế độ sang Turbo (ép quạt 100% hoặc kích hoạt thanh ghi Turbo WMI/EC), cập nhật viền sáng sang nút Turbo.
ProfileSilent	Chuyển chế độ sang Silent (giới hạn trần quạt êm ái theo ngưỡng an toàn), cập nhật giao diện.
ToggleEpp	Đảo trạng thái khóa xung/nhiệt độ Smooth EPP Governor (Bật <-> Tắt), áp dụng ngay chính sách nguồn điện CPU và đổi nhãn nút giữa [ ĐANG BẬT ] (màu Cyan) và [ ĐANG TẮT ] (màu Xám).
OpenCurveEditor	Mở file cấu hình config.toml bằng trình soạn thảo mặc định của hệ thống (ShellExecuteW với lệnh "open") hoặc mở hộp thoại chỉnh nhanh, giúp người dùng sửa điểm uốn đồ thị quạt tức thì.
OpenTelemetryChart	Gọi trực tiếp hàm mở cửa sổ Biểu đồ nhiệt độ 1h (chart_window::show_chart_window()).
ReloadConfig	Gọi hàm đọc lại config.toml ngay lập tức, cập nhật Fan Curve & EPP, đặt reload_flash_ticks = 3 để hiển thị dòng chữ xanh "✓ Đã tải lại cấu hình thành công!" trên Header trong 3 giây.
ExitApp	Ẩn cửa sổ, tháo Tray Icon, khôi phục quạt về chế độ Auto của BIOS để bảo vệ phần cứng, và gọi PostQuitMessage(0) kết thúc toàn bộ ứng dụng.
ToggleTheme	Chuyển đổi qua lại giữa Predator Dark (#121417) và Light Mode, gọi InvalidateRect để đổi giao diện tức thời.
3. TIÊU CHUẨN NGHIỆM THU & KIỂM THỬ (QA & VERIFICATION CHECKLIST CHO ANTIGRAVITY)

Sau khi viết code xong, Antigravity cần chạy bộ kiểm thử sau và ghi kết quả vào báo cáo Markdown:

1
Kiểm tra Biên dịch & Kích thước Binary
Cargo Release Build

Chạy cargo build --release. Đảm bảo trình biên dịch hoàn tất với 0 errors, 0 warnings và kích thước file .exe giữ nguyên dưới ngưỡng 3 MB.

2
Kiểm tra Tương tác Khay hệ thống & Singleton
Tray Left/Right Click

Click chuột trái vào icon dưới System Tray -> Cửa sổ Dashboard phải bật lên ngay giữa màn hình. Click chuột trái lần nữa hoặc khi cửa sổ đang bị che khuất -> Cửa sổ phải tự động nhảy lên trên cùng (SetForegroundWindow) mà không tạo ra cửa sổ thứ hai. Click chuột phải -> Menu truyền thống vẫn hoạt động bình thường.

3
Kiểm tra Phản hồi Xúc giác (Tactile Feedback)
Hover, Press & Release

Rê chuột qua từng nút (Active Curve, Turbo, Silent, EPP Toggle, 4 nút công cụ) -> Nút phải sáng nền lên mượt mà, không nhấp nháy. Nhấn giữ chuột trái -> Nút phải lún xuống 2px. Kéo chuột ra ngoài rồi thả -> Không kích hoạt nhầm lệnh.

4
Kiểm tra Hiệu năng & Chống rò rỉ GDI Objects
RAM dưới 15MB, CPU 0%

Mở Task Manager (Details tab), bật cột GDI Objects, Memory (Active Private Working Set) và CPU:

Rê chuột liên tục khắp cửa sổ trong 30 giây -> Số lượng GDI Objects phải giữ nguyên cố định (tuyệt đối không tăng dần theo thời gian).

Bộ nhớ RAM tiêu thụ tổng cộng phải nhỏ hơn 15 MB (kỳ vọng đạt 2.5 MB - 5 MB).

Nhấn nút [X] đóng cửa sổ Dashboard -> Cửa sổ ẩn xuống Tray, mức sử dụng CPU lập tức rơi về 0.00%.

4. YÊU CẦU BÁO CÁO TIẾN ĐỘ TỪ ANTIGRAVITY

Sau khi hoàn thành thi công theo kế hoạch trên, Antigravity hãy tổng hợp báo cáo tiến trình (bao gồm danh sách file đã sửa đổi/tạo mới, kết quả biên dịch cargo build --release, và thông số đo đạc RAM/GDI Objects thực tế) để 4 Argon nghiệm thu hoặc tinh chỉnh sâu hơn nếu cần.