<!-- Kế hoạch được lập bởi: Gemini 4 Argon (Cao) -->

BÁO CÁO KỸ THUẬT & KẾ HOẠCH TRIỂN KHAI CHI TIẾT
Khắc phục triệt để chữ mờ, nhòe và chuẩn hóa cỡ chữ / tương phản UI (predator-fan-ctl)

Phân tích phản hồi từ người dùng cho thấy giao diện cửa sổ Predator Fan Control // Dashboard bị mờ, nhòe và chữ quá nhỏ là do sự kết hợp của 4 nguyên nhân cốt lõi: cơ chế DPI Virtualization của Windows DWM trên màn hình 125% DPI (120 DPI), cách tính toán font bằng Cell Height thay vì Em-Height trong GDI, độ tương phản màu chữ phụ (text_sec) chưa đạt chuẩn, và kích thước cửa sổ chưa co giãn theo hệ số DPI thực tế.

Dưới đây là phương án kỹ thuật toàn diện và Master Implementation Plan (plan.md) nhằm khắc phục triệt để hiện tượng này, nâng cấp độ sắc nét lên mức tuyệt đối (Pixel-Perfect Subpixel Rendering).

I. Chi tiết Phương án Kỹ thuật Khắc phục
1. Kích hoạt PerMonitorV2 DPI Awareness (Loại bỏ Bitmap Stretching)

Vấn đề: Do ứng dụng thuần Win32 C/Rust chạy mà không có file manifest nhúng DPI Awareness, Windows coi tiến trình là DPI-Unaware (96 DPI) và tự động dùng thuật toán bilinear co giãn màn hình, phá hủy công nghệ khử răng cưa ClearType.

Giải pháp: Ngay tại dòng đầu tiên của hàm main() trong src/main.rs, thực hiện gọi API hệ thống thuần qua Win32 COM/User32:

Rust
unsafe {
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
}


Kết quả: Hệ thống DWM không còn scale ảo bitmap, toàn bộ cửa sổ GDI được render trực tiếp tại độ phân giải gốc của màn hình 2560x1440, mang lại độ sắc nét hoàn hảo đến từng subpixel.

2. Chuẩn hóa hàm tạo Font Win32 GDI DPI-Aware (-MulDiv(pt, dpi, 72))

Vấn đề: Truyền height > 0 vào CreateFontW tạo ra Cell Height (bao gồm khoảng đệm font), khiến chữ bị nhỏ hơn thiết kế thực tế.

Giải pháp: Viết lại hàm khởi tạo font tự động truy vấn DPI động của cửa sổ (GetDpiForWindow) và áp dụng công thức tính Em-Height chính xác theo Point Size:

Rust
unsafe fn create_dpi_font(hwnd: HWND, point_size: i32, weight: i32, face_name: &[u16]) -> HFONT {
    let dpi = GetDpiForWindow(hwnd);# BÁO CÁO KỸ THUẬT & KẾ HOẠCH TRIỂN KHAI (PLAN.md)

KHẮC PHỤC CHỮ MỜ, NHÒE & TĂNG KÍCH THƯỚC ĐỘ RÕ NÉT GIAO DIỆN UI (WIN32 GDI NATIVE)

Chào bạn, dựa trên các thông số thực tế của hệ thống (LAPTOP-QBELO7HH, màn hình độ phân giải 2560x1440