# BÁO CÁO HIỆN TRẠNG & YÊU CẦU: KHẮC PHỤC CHỮ MỜ, NHÒE VÀ TĂNG KÍCH THƯỚC ĐỘ RÕ NÉT GIAO DIỆN UI (WIN32 GDI NATIVE)

## 1. Yêu cầu của người dùng
- **Nội dung yêu cầu**: "giao diện UI chữ bị mờ, nhòe, không rõ chữ. Sữa lại cho to rõ hơn" kèm theo ảnh chụp màn hình thực tế của cửa sổ `Predator Fan Control // Dashboard`.
- **Mục tiêu**:
  1. Loại bỏ triệt để hiện tượng chữ bị mờ, nhòe (DPI blur / bitmap stretching).
  2. Tăng kích thước chữ (Font Size & Weight) cho toàn bộ các thành phần trên UI: tiêu đề, chỉ số CPU/GPU Telemetry (Nhiệt độ, Công suất W, Xung nhịp GHz, Tốc độ quạt % và RPM), các nút bấm Profile, thanh Governor, và các nút điều hướng chức năng.
  3. Nâng cao độ tương phản màu sắc (Color Contrast) của font chữ phụ (`text_sec`), giúp người dùng đọc rõ ràng ngay cả khi nhìn lướt ở khoảng cách xa hoặc trên màn hình độ phân giải cao 2K/4K.

---

## 2. Phân tích Nguyên nhân Kỹ thuật Gốc rễ (Root Cause Analysis)

### Nguyên nhân 1: Ứng dụng bị Windows DWM cưỡng chế co giãn Bitmap (DPI Virtualization Bitmap Scaling)
- **Đo đạc thực tế**: Màn hình chính của laptop người dùng có độ phân giải **2560x1440 với mức thu phóng Display Scaling là 125% (DPI = 120)**.
- **Vấn đề cốt lõi**:
  + Do ứng dụng chạy thuần Win32 C/Rust và trước đó đã gỡ bỏ khối manifest xung đột với linker MinGW, Windows coi tiến trình là **DPI Unaware (96 DPI)**.
  + Khi một tiến trình DPI-Unaware hiển thị trên màn hình 125% DPI (120 DPI), hệ thống Windows DWM sẽ tự động vẽ ứng dụng vào một buffer ảo 96 DPI, sau đó **dùng thuật toán phóng to bilinear (Bilinear Scaling / Bitmap Stretch) lên 125%**.
  + Quá trình phóng to ảnh bitmap này làm phá hủy hoàn toàn công nghệ khử răng cưa điểm ảnh phụ **ClearType subpixel antialiasing**, biến toàn bộ nét chữ Segoe UI và các đường viền sắc nét thành các vệt mờ, nhòe, lem viền đặc trưng.

### Nguyên nhân 2: Phương thức khởi tạo Font trong GDI dùng Chiều cao Ô (Cell Height) thay vì Chiều cao Ký tự (Character Em-Height)
- Trong `src/dashboard_window.rs`:
  ```rust
  unsafe fn create_font(height: i32, weight: i32) -> HFONT {
      CreateFontW(height, 0, 0, 0, weight, ..., CLEARTYPE_QUALITY, ..., w!("Segoe UI"))
  }
  ```
  + Trong Win32 GDI, khi truyền `height > 0` vào `CreateFontW`, GDI hiểu đó là **Cell Height** (bao gồm cả khoảng đệm đỉnh internal leading). Do đó `height = 13` chỉ tạo ra ký tự thực tế cao ~9-10 device pixels (tương đương cỡ chữ ~7.5 pt siêu nhỏ!).
  + Chuẩn Win32 GDI yêu cầu truyền **giá trị âm** `-MulDiv(point_size, dpi, 72)` để chỉ định chính xác chiều cao ký tự (Em-Height) theo Point Size thực tế của nhà thiết kế.

### Nguyên nhân 3: Độ tương phản màu sắc của Font chữ phụ (`text_sec`) quá thấp
- Bảng màu `AppTheme::Dark`:
  `text_sec: rgb(139, 148, 158)` (#8B949E) đặt trên nền card `card: rgb(22, 25, 30)` (#16191E).
  Tỷ lệ tương phản chỉ đạt ~4.2:1, gây khó đọc đối với các thông số kỹ thuật dài như `"Công suất: 41.1 W | Xung nhịp: 2.60 GHz"` và `"Quạt: 62% | 4718 RPM"`.
- Cần nâng màu `text_sec` lên mức sáng và rõ ràng hơn: `#C8D1DC` hoặc `#D5DFEC` (tương phản > 9:1).

### Nguyên nhân 4: Kích thước cửa sổ mặc định và padding chưa tương thích với Font lớn
- Cửa sổ Dashboard hiện tại có kích thước cố định ban đầu là `540 x 450 px` (chuẩn 96 DPI). Khi chạy ở 120 DPI hoặc khi tăng cỡ chữ lên 1.25x - 1.4x, các card và nút bấm cần được tính toán không gian co giãn linh hoạt để không bị tràn chữ hoặc cụt dòng.

---

## 3. Đề xuất Giải pháp Kỹ thuật Toàn diện

1. **Kích hoạt PerMonitorV2 DPI Awareness bằng Win32 API thuần (Zero-Manifest Collision)**:
   - Tại dòng đầu tiên của hàm `main()` trong `src/main.rs`:
     Gọi `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)`.
   - Giải pháp này kích hoạt trực tiếp cấp độ DPI Awareness cao nhất của Windows 10/11 qua API runtime mà hoàn toàn không cần file manifest tĩnh, tránh 100% nguy cơ xung đột với linker GNU `ld.exe`.
   - Kết quả: DWM ngừng co giãn bitmap, toàn bộ chữ và hình khối được rasterize trực tiếp ở độ phân giải gốc 2560x1440, sắc nét tuyệt đối đến từng subpixel!

2. **Chuẩn hóa hàm tạo Font DPI-Aware (`CreateFontW` với `-MulDiv(pt, dpi, 72)`)**:
   - Truy vấn DPI thực tế của cửa sổ: `GetDpiForWindow(hwnd)` (mặc định 96 nếu hàm không sẵn sàng).
   - Nâng cấp kích thước font:
     + **Header Title**: Từ 17 cell px -> 16 pt Bold (`-MulDiv(16, dpi, 72)`).
     + **Card Header ("CPU/GPU TELEMETRY")**: Từ 14 cell px -> 13 pt Bold.
     + **Nhiệt độ chính ("67°C")**: Từ 36 cell px -> 34 pt Bold (rõ ràng, nổi bật).
     + **Thông số Công suất & Xung nhịp**: Từ 13 cell px -> 12 pt Semi-Bold với màu chữ sáng `#D5DFEC`.
     + **Nhãn RPM & Tốc độ Quạt**: Từ 13 cell px -> 12 pt Semi-Bold.
     + **Nút bấm chức năng (Profile, Governor, Công cụ)**: Từ 14 cell px -> 13.5 pt Bold/Semi-Bold.

3. **Cải tiến độ tương phản Theme Palette**:
   - Tối ưu màu `text_sec` ở Dark Mode từ `#8B949E` lên `#C8D3E0`.
   - Tối ưu màu `text` chính lên `#FFFFFF` hoặc `#F0F4F8`.
   - Tối ưu màu sắc hiển thị trên nút bấm và thanh tiến trình quạt.

4. **Nâng cấp Layout & Kích thước cửa sổ ban đầu**:
   - Kích thước cửa sổ khởi tạo được nhân theo tỷ lệ DPI: `width = MulDiv(580, dpi, 96); height = MulDiv(480, dpi, 96)`.
   - Giới hạn tối thiểu `WM_GETMINMAXINFO` được điều chỉnh thích ứng với DPI.

5. **Đồng bộ trên cả Biểu đồ 1 Giờ (`chart_window.rs`) và Hộp thoại Curve (`curve_dialog.rs`)**:
   - Đảm bảo toàn bộ hệ thống giao diện trong bộ phần mềm đều đạt chuẩn sắc nét PerMonitorV2 tương tự.

---

Nhờ Kiến trúc sư trưởng (Gemini Master Planner) đánh giá tình hình, phân tích giải pháp kỹ thuật và ban hành Kế hoạch Triển khai Chi tiết (`plan.md`).
