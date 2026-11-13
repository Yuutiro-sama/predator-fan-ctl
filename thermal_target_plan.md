Bản phân tích kỹ thuật và Kế hoạch triển khai tổng thể (Master Implementation Plan) dưới đây giải quyết bài toán kiểm soát nhiệt độ mục tiêu (Thermal Target 80°C ± 3°C) trên Acer Predator Helios Neo 16 (PHN16-71) trang bị Intel Core i7-13650HX (Raptor Lake-HX) và BIOS InsydeH2O v1.20, đồng thời định hình kiến trúc kỹ thuật để kỹ sư Antigravity trực tiếp thi công.

PHẦN 1: PHÂN TÍCH KỸ THUẬT CHUYÊN SÂU (ARCHITECTURAL & HARDWARE ANALYSIS)
1. Bản chất cơ chế "Temperature Limit" trên Lenovo Legion Zone / Legion Toolkit

Khi người dùng chọn giới hạn nhiệt độ (ví dụ 80°C) trong Custom Mode của Lenovo Legion Zone hoặc Lenovo Legion Toolkit, hệ thống không dùng vòng lặp phần mềm chậm chạp để cắt giảm xung nhịp từ tầng OS, mà can thiệp trực tiếp vào vi kiến trúc quản lý năng lượng của Intel và Embedded Controller (EC) qua 2 cơ chế song song:

Cơ chế 1 — Điều chỉnh ngưỡng TCC Activation Offset (PROCHOT Offset) ở cấp độ Silicon:

Trên CPU Intel dòng HX (Raptor Lake-HX), nhiệt độ giới hạn tối đa mặc định (
𝑇
𝑗
𝑢
𝑛
𝑐
𝑡
𝑖
𝑜
𝑛
_
𝑚
𝑎
𝑥
T
junction_max
	​

 hay 
𝑇
𝑗
𝑀
𝑎
𝑥
T
jMax
	​

) là 100°C.

Intel trang bị thanh ghi MSR 0x1A2 (MSR_TEMPERATURE_TARGET). Các bit [29:24] của thanh ghi này lưu giá trị TCC Activation Offset (Thermal Control Circuit Offset), cho phép dịch chuyển điểm kích hoạt giảm xung nhiệt (PROCHOT#) xuống thấp hơn 
𝑇
𝑗
𝑀
𝑎
𝑥
T
jMax
	​

.

Khi người dùng đặt giới hạn 80°C trên Legion Zone, EC/BIOS của Lenovo ghi giá trị Offset = 20 (
100
−
20
=
80
∘
C
100−20=80
∘
C) vào thanh ghi MSR 0x1A2 (hoặc thông qua giao thức PECI từ EC sang CPU). Ngay khi bất kỳ nhân nào chạm 80°C, mạch phần cứng P-Unit (Power Control Unit) bên trong đế silicon của i7-13650HX sẽ tự động giảm điện áp (
𝑉
𝑐
𝑜
𝑟
𝑒
V
core
	​

) và xung nhịp theo chu kỳ nano-giây/micro-giây để ghim nhiệt độ đứng im tại 80°C (±1°C).

Cơ chế 2 — Nội suy động PL1/PL2 qua Lenovo EC & Intel DTT (Dynamic Tuning Technology):

Song song với TCC Offset, chip EC của Lenovo liên tục điều chỉnh MMIO MCHBAR (0xFED159A0 - Package RAPL Limit) để hạ mượt mà ngưỡng công suất ngắn hạn (
𝑃
𝐿
2
PL2) và dài hạn (
𝑃
𝐿
1
PL1) khi nhiệt độ tiệm cận 80°C, tránh hiện tượng xung nhịp bị răng cưa (oscillation) khi đập thẳng vào tường PROCHOT#.

2. Thực trạng trên Acer Predator Helios Neo 16 (PHN16-71 - BIOS InsydeH2O v1.20)
A. Phần mềm PredatorSense

Hoàn toàn KHÔNG hỗ trợ tính năng tùy chỉnh giới hạn nhiệt độ mục tiêu (Custom Thermal Target) hay chỉnh tay 
𝑃
𝐿
1
/
𝑃
𝐿
2
PL1/PL2.

PredatorSense chỉ cung cấp 4 profile cứng (Quiet, Balanced, Performance, Turbo) nạp sẵn bảng công suất 
𝑃
𝐿
1
/
𝑃
𝐿
2
PL1/PL2 và ngưỡng TCC cố định trong EC (thường để 
𝑇
𝑗
𝑀
𝑎
𝑥
=
100
∘
C
T
jMax
	​

=100
∘
C hoặc 
92
∘
C
92
∘
C tùy profile, dẫn đến việc CPU liên tục vọt lên 95°C–100°C khi chạy tải nặng).

B. Nền tảng phần cứng Intel Core i7-13650HX

Về mặt silicon, i7-13650HX hỗ trợ 100% cả việc chỉnh TCC Offset (MSR 0x1A2) lẫn điều chỉnh công suất tức thời qua RAPL (Running Average Power Limit) tại thanh ghi MSR 0x610 (MSR_PKG_POWER_LIMIT) và vùng nhớ MMIO 0xFED159A0.

C. Rào cản từ BIOS InsydeH2O v1.20 của Acer

Trên phiên bản BIOS v1.20 của PHN16-71, Acer áp đặt các hàng rào khóa phần cứng sau trong quá trình POST:

Undervolt Protection (UVP) & Overclocking Lock (MSR 0x194 bit 20): Bị khóa cứng ở mức 1, chặn hoàn toàn việc hạ điện áp (Undervolt) qua MSR 0x150 trong Windows.

TCC Offset Lock (MSR 0x1A2 bit 31): Trong menu BIOS tiêu chuẩn của Acer không có mục mở khóa. Tùy thuộc vào bảng cấu hình khởi tạo (DxePlatform) của bản BIOS v1.20, bit 31 của MSR 0x1A2 hoặc thanh ghi MMIO tương ứng có thể ở trạng thái mở (0) hoặc khóa (1), hoặc bị trình điều khiển Intel Dynamic Tuning Technology (DTT / IPF) trong Windows liên tục ghi đè mỗi vài giây.

RAPL MMIO (0xFED159A0) & MSR (0x610) Lock: Hầu hết các dòng PHN16-71 KHÔNG khóa bit 63 (Lock bit) của thanh ghi MSR 0x610 và vùng nhớ MMIO 0xFED159A0 (để phục vụ việc chuyển đổi profile giữa Quiet/Balanced/Turbo của PredatorSense lúc máy đang chạy). Đây chính là "cửa hậu" kỹ thuật cực kỳ quan trọng để can thiệp bằng phần mềm.

3. Phân tích chuyên sâu 4 Giải pháp khả thi (Target: 80°C ± 3°C)
Giải pháp A: Can thiệp TCC Offset & RAPL trực tiếp qua MSR / ThrottleStop

Cơ chế: Ghi giá trị 20 (tương ứng 
100
∘
C
−
20
∘
C
=
80
∘
C
100
∘
C−20
∘
C=80
∘
C) vào các bit [29:24] của thanh ghi MSR 0x1A2 (MSR_TEMPERATURE_TARGET).

Điều kiện cần: Bit 31 (TCC Offset Lock) của MSR 0x1A2 phải bằng 0, và dịch vụ Intel(R) Innovation Platform Framework (IPF) / Dynamic Tuning không tự động ghi đè lại giá trị mặc định.

Cách kiểm tra nhanh:

Dùng ThrottleStop (v9.6+): Mở cửa sổ TPL (Turbo Power Limits) -> nhìn vào mục PROCHOT Offset.

Nếu KHÔNG có biểu tượng ổ khóa (Lock icon) bên cạnh PROCHOT Offset: Có thể nhập trực tiếp số 20, tích chọn Lock PROCHOT Offset, bấm Apply. Nhiệt độ sẽ lập tức bị khóa cứng ở đúng 80°C.

Nếu CÓ biểu tượng ổ khóa: BIOS v1.20 đã set bit 31 = 1 tại thời điểm POST, không thể sửa MSR 0x1A2 từ Windows nếu không mở khóa BIOS hoặc chuyển sang điều khiển qua đường PL1/PL2.

Giải pháp B: Mở khóa BIOS ẩn (Advanced BIOS / SREP / UMAF) để chỉnh TCC Offset từ Firmware

Cơ chế: Trên PHN16-71 với BIOS InsydeH2O v1.20, Acer đã vô hiệu hóa tổ hợp phím Fn + Tab truyền thống. Để vào menu ẩn (Intel Advanced Menu -> Thermal Configuration -> TCC Activation Offset), phải dùng công cụ SREP (Smokeless Runtime EFI Patcher) boot từ USB FAT32 (tắt Secure Boot) để nạp tập tin cấu hình patch bộ nhớ NVRAM/HII của InsydeH2O trong phiên khởi động.

Thiết lập trong BIOS ẩn:

Thermal Configuration -> CPU Thermal Configuration -> Tcc Activation Offset = 20.

Tcc Offset Lock = Disabled.

Rủi ro: Cao. Thao tác sai trong SREP hoặc chỉnh nhầm thông số P-State/Ring Voltage trên BIOS v1.20 có thể gây brick BIOS (màn hình đen, phải dùng kẹp CH341A nạp lại ROM hoặc mang ra trung tâm bảo hành). Không phù hợp nếu ưu tiên sự an toàn và tiện lợi.

Giải pháp C: Bộ điều khiển công suất động vòng kín (Closed-Loop Dynamic Thermal Governor) tích hợp trực tiếp vào App Rust (predator-fan-ctl)

Đây là giải pháp tối ưu, an toàn và tương đương 99% với Lenovo Legion Zone mà không cần động chạm hay hack BIOS.

Nguyên lý hoạt động: Thay vì đợi CPU chạm 80°C mới kích hoạt xung PROCHOT, ứng dụng chạy nền (vốn đã chạy với quyền requireAdministrator trong dự án predator-fan-ctl của bạn) sẽ đóng vai trò một PID / PI Thermal Controller hoạt động theo chu kỳ 250ms – 500ms:

Đọc nhiệt độ CPU thực tế (
𝑇
𝑐
𝑢
𝑟
𝑟
𝑒
𝑛
𝑡
T
current
	​

) từ WMI ACPI Acer hoặc cảm biến nhiệt độ nhân.

Tính toán sai số nhiệt độ: 
𝑒
(
𝑡
)
=
𝑇
𝑡
𝑎
𝑟
𝑔
𝑒
𝑡
−
𝑇
𝑐
𝑢
𝑟
𝑟
𝑒
𝑛
𝑡
e(t)=T
target
	​

−T
current
	​

 (với 
𝑇
𝑡
𝑎
𝑟
𝑔
𝑒
𝑡
=
80
∘
C
T
target
	​

=80
∘
C).

Khi 
𝑇
𝑐
𝑢
𝑟
𝑟
𝑒
𝑛
𝑡
T
current
	​

 vượt quá 
77
∘
C
77
∘
C (bắt đầu vào vùng 
80
∘
C
±
3
∘
C
80
∘
C±3
∘
C), thuật toán PI tự động điều chỉnh giảm dần công suất trần (Dynamic PL1/PL2) hoặc Maximum Processor Frequency / EPP theo thời gian thực.

Khi nhiệt độ hạ xuống dưới 
78
∘
C
78
∘
C, thuật toán nới lỏng trần công suất lên lại để tận dụng tối đa hiệu năng.

3 Kênh điều khiển phần cứng (Actuators) từ Rust trên Windows:

Kênh 1 (Chuẩn xác nhất - Mức Watt): Ghi trực tiếp vào thanh ghi RAPL MMIO 0xFED159A0 và MSR 0x610 để chỉnh 
𝑃
𝐿
1
/
𝑃
𝐿
2
PL1/PL2 theo từng bước 
1
W
1W (trong dải từ 
25
W
25W đến 
115
W
115W).

Kênh 2 (Không cần Driver Kernel Ring-0 - Native Win32 API): Gọi hàm PowerWriteACValueIndex / PowerApplySettingChanges của thư viện powrprof.dll (Win32) tác động vào GUID PROCTHROTTLEMAX (bc5038f7-23e0-4960-96da-33abaf5935ec - Giới hạn % xung nhịp tối đa) hoặc PERFEPP (36687f9e-e3a5-4dbf-b1dc-15eb381c6863 - Energy Performance Preference). Việc調 chỉnh xung nhịp trần theo bước 100MHz hoặc % qua powrprof.dll có hiệu lực ngay lập tức (< 10ms) trên Windows 11 mà không bị bất kỳ khóa BIOS nào chặn!

Kênh 3 (Nếu MSR 0x1A2 không khóa): Ghi thẳng TCC Offset = 20 vào MSR 0x1A2.

Giải pháp D: Khống chế PL1/PL2 tĩnh (Static Sweet-Spot Power Capping)

Cơ chế: Trên hệ thống tản nhiệt 2 quạt AeroBlade 3D + Liquid Metal của PHN16-71 (i7-13650HX), công suất tỏa nhiệt tương ứng với mức nhiệt ổn định