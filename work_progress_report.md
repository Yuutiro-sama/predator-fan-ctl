# BÁO CÁO HIỆN TRẠNG & ĐỀ XUẤT KẾ HOẠCH: UP MÃ NGUỒN LÊN GITHUB

## 1. Yêu cầu của người dùng
1. Tự động khởi tạo Repository mới trên GitHub dưới tài khoản người dùng (`Yuutiro-sama`).
2. Tự động đẩy (push) toàn bộ mã nguồn lên GitHub.
3. Soạn thảo tài liệu `README.md` song ngữ (VI / EN) chuyên nghiệp, đầy đủ kiến trúc, tính năng, hướng dẫn cài đặt và sử dụng.
4. Bổ sung Giấy phép mã nguồn mở MIT (`LICENSE`).
5. Phần Lưu ý bắt buộc trong README và Release:
   - **Tuyên bố về AI (AI Disclosure & Transparency)**: Làm rõ vai trò phát triển mã nguồn, kiến trúc và kiểm thử bởi AI (Google Antigravity & Gemini) phối hợp cùng người dùng.
   - **Tính thương mại & Miễn trừ trách nhiệm (Commercial Use & Disclaimer)**: Tuyên bố quyền sử dụng phi thương mại / tự do học tập nghiên cứu, miễn trừ rủi ro can thiệp phần cứng EC, nghiêm cấm trục lợi thương mại đóng gói trái phép.
6. Tạo GitHub Release chính thức (`v1.5.0`) đính kèm bản cài đặt `dist/predator-fan-ctl-setup-v1.5.0.exe` và Release Notes song ngữ.
7. **Ràng buộc an toàn màn hình**: Toàn quyền sử dụng màn hình phụ (màn 1: B160UAN01.Q, tọa độ x: -1920..0), tuyệt đối KHÔNG chạm hoặc tương tác vào màn hình chính của người dùng (màn 2: AG2313QS1, tọa độ x: 0..2560 nơi đang chạy game).

## 2. Hiện trạng hệ thống & Môi trường thực tế
- Thư mục làm việc: `c:\Users\default.LAPTOP-1V8CRH32\Documents\antigravity\fan-control`
- Git local repo: Đã khởi tạo, commit sạch sẽ, branch hiện tại là `master`.
- Tài khoản GitHub:
  - Username: `Yuutiro-sama`
  - Email: `dangtanmin111@gmail.com`
  - Chứng thực: OAuth Token GitHub đã lưu trong Windows Credential Manager (`git:https://github.com`), API trả về hợp lệ.
- File cài đặt đã hoàn thiện: `dist\predator-fan-ctl-setup-v1.5.0.exe` (đã nén và kiểm thử hoạt động).
- Màn hình hiển thị:
  - Màn chính (AG2313QS1): Bounds `(0, 0, 2560, 1440)` -> CẤM ĐỤNG VÀO.
  - Màn phụ (B160UAN01.Q): Bounds `(-1920, 0, 1920, 1200)` -> Toàn quyền sử dụng.
