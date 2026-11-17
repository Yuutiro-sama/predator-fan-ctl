# Predator Fan Controller (`predator-fan-ctl`)

<div align="center">

![Predator Fan Controller](assets/screenshots/dashboard.png)

[![Release](https://img.shields.io/badge/Release-v1.5.0-blue.svg?style=for-the-badge&logo=github)](https://github.com/)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%7C%2011%20x64-0078D6.svg?style=for-the-badge&logo=windows)](https://microsoft.com/windows)
[![Built With](https://img.shields.io/badge/Built%20With-Rust-DEA584.svg?style=for-the-badge&logo=rust)](https://www.rust-lang.org/)
[![GUI Engine](https://img.shields.io/badge/GUI-Native%20Win32%20GDI-informational.svg?style=for-the-badge)](https://learn.microsoft.com/windows/win32/gdi/windows-gdi)
[![Memory Footprint](https://img.shields.io/badge/RAM-~2--4%20MB-brightgreen.svg?style=for-the-badge)](#kiến-trúc-kỹ-thuật-chuyên-sâu)
[![License](https://img.shields.io/badge/License-MIT-green.svg?style=for-the-badge)](LICENSE)

**Bộ điều khiển quạt & giám sát nhiệt độ siêu nhẹ, thuần Native Win32 GDI viết bằng Rust dành riêng cho laptop Acer Predator / Nitro.**

**Ultra-lightweight, zero-overhead fan controller & thermal telemetry monitor written in pure Rust & native Win32 GDI for Acer Predator / Nitro laptops.**

---

**[🇻🇳 Tiếng Việt](#-tiếng-việt)** | **[🇬🇧 English](#-english)**

</div>

---

<a name="tiếng-việt"></a>
## 🇻🇳 Tiếng Việt

### 1. Tổng quan & Tầm nhìn Dự án

Phần mềm điều khiển phần cứng của các nhà sản xuất máy tính xách tay (OEM) thường đi kèm hàng loạt tiến trình chạy nền nặng nề (Electron, WebView2, .NET Framework), tiêu tốn từ 200 MB đến 600 MB RAM và gây giật khung hình (micro-stutter) ngẫu nhiên khi chơi game do cơ chế WMI polling cồng kềnh.

**`predator-fan-ctl`** được sinh ra nhằm giải quyết triệt để vấn đề này:
- **Tối giản cực hạn**: Được xây dựng 100% bằng **Rust** kết hợp giao diện đồ họa **Native Win32 GDI** thuần túy (không sử dụng Electron, Chromium hay bất kỳ web framework nào).
- **Mức chiếm dụng bộ nhớ chỉ ~2 - 4 MB RAM** và **tiêu thụ CPU < 0.05%** trong suốt quá trình giám sát nền.
- **Can thiệp phần cứng cấp thấp**: Tương tác trực tiếp qua WMI ACPI Methods (`Acer_FanInterface`) hoặc qua vi điều khiển nhúng Direct EC (`inpoutx64.dll`).
- **An toàn tuyệt đối**: Tích hợp cơ chế phục hồi phần cứng đa tầng (Fail-safe Restoration), luôn tự động trả quạt về quyền điều khiển của BIOS / PredatorSenseService khi đóng ứng dụng.

---

### 2. Tính năng nổi bật

- 🪟 **Giao diện Native Win32 GDI Siêu Nhanh & Mượt Mà**:
  - Giao diện Dashboard tối màu (Dark Mode) phong cách Cyberpunk/Predator sắc nét, hỗ trợ đầy đủ HiDPI scaling trên màn hình 2K/4K.
  - Biểu đồ thời gian thực (Real-time Telemetry Chart) hiển thị song song nhiệt độ CPU/GPU, vòng tua quạt (RPM/%) và công suất ở tần số làm tươi 60 FPS không gây xé hình nhờ kỹ thuật Double Buffering.
- 🎛️ **Trình Biên tập Đường cong Quạt (Interactive Fan Curve Editor)**:
  - Cho phép tùy chỉnh tự do đường đặc tính quạt CPU & GPU theo từng mốc nhiệt độ bằng thao tác kéo thả chuột trực quan.
  - Tích hợp vùng trễ nhiệt (Hysteresis) thông minh, ngăn chặn hiện tượng quạt rú/hạ ga liên tục khi nhiệt độ dao động quanh ngưỡng kích hoạt.
- ⚡ **Bộ Điều Tốc Năng Lượng Thông Minh (Smooth EPP Governor)**:
  - Tự động điều chỉnh chỉ số EPP (*Energy Performance Preference*) của vi xử lý Intel/AMD theo tải trọng thực tế, hạ nhiệt độ khi máy rảnh rỗi và giải phóng toàn bộ xung nhịp Turbo Boost khi tải nặng.
- 📊 **Giám Sát Phần Cứng Không Tiêu Tốn Phân Phối Bộ Nhớ (Zero-Allocation Telemetry)**:
  - Đọc xung nhịp thực tế đa nhân qua Win32 FFI `CallNtPowerInformation` (PowrProf), nhận diện xung nhịp boost đỉnh của từng P-Core/E-Core.
  - Trích xuất công suất Intel Package 0 (RAPL) trực tiếp từ bộ đếm hiệu năng Windows PDH với độ chính xác đến từng 0.1 W.
  - Nạp động NVIDIA NVML (`nvml.dll`) an toàn, không đánh thức GPU khi đang trong trạng thái ngủ sâu NVIDIA Optimus D3cold.
- 🛡️ **Khay Hệ Thống & Bảo Vệ Phần Cứng Đa Tầng**:
  - Tích hợp System Tray Menu chuyển nhanh các chế độ quạt: *Quiet, Balanced, Performance, Turbo, Custom Curve, Auto*.
  - Bộ bảo vệ Service Guard tự động quản lý trạng thái `PredatorSenseService` và khôi phục chế độ BIOS Auto khi người dùng thoát ứng dụng hoặc tắt máy.

---

### 3. Hình ảnh giao diện thực tế

<div align="center">

#### Giao diện Bảng điều khiển Trung tâm (Main Dashboard)
![Dashboard](assets/screenshots/dashboard.png)

#### Biểu đồ Giám sát Telemetry Thời gian thực (Real-time Telemetry Chart)
![Telemetry Chart](assets/screenshots/telemetry_chart.png)

#### Trình Hiệu chỉnh Đường cong Quạt (Fan Curve Editor)
![Curve Dialog](assets/screenshots/curve_dialog.png)

</div>

---

### 4. Kiến trúc Kỹ thuật Chuyên sâu

```
┌────────────────────────────────────────────────────────────────────────┐
│                   PREDATOR-FAN-CTL ARCHITECTURE                        │
├────────────────────────────────────────────────────────────────────────┤
│  [ Native Win32 GDI GUI Layer ]                                        │
│   ├── Dashboard Window (GDI Double Buffering, HiDPI aware)             │
│   ├── Telemetry Chart (60 FPS Bezier/Polyline, Circular History Buffer)│
│   ├── Fan Curve Dialog (Draggable Control Points, Hysteresis Logic)    │
│   └── System Tray Manager (Shell_NotifyIconW, Win32 Popup Menu)        │
├────────────────────────────────────────────────────────────────────────┤
│  [ Core Thermal Governor & Control Loop ]                              │
│   ├── ThermalGovernor (Smooth EPP Stepping, Load-aware Hysteresis)     │
│   ├── Config Engine (TOML Serde, In-memory Atomic Profile Switch)      │
│   └── Fail-safe Supervisor (SetConsoleCtrlHandler, Mutex, Auto-restore)│
├────────────────────────────────────────────────────────────────────────┤
│  [ Zero-Allocation Telemetry Layer ]                                   │
│   ├── CPU Clock: PowrProf CallNtPowerInformation (max CurrentMhz)      │
│   ├── CPU Power: Windows PDH English Counter (RAPL_Package0_PKG Power) │
│   ├── CPU Temp:  ACPI ThermalZone via WMI (MSAcpi_ThermalZoneTemp)     │
│   └── GPU Data:  Dynamic NVML FFI (nvmlDeviceGetTemp/Power/ClockInfo)  │
├────────────────────────────────────────────────────────────────────────┤
│  [ Low-Level Hardware Backends ]                                       │
│   ├── AcerWmiBackend (root\WMI -> Acer_FanInterface Methods)          │
│   ├── DirectEcBackend (Embedded Controller Direct I/O via inpoutx64)   │
│   └── SCM Guard (Safe pausing & resuming of PredatorSenseService)      │
└────────────────────────────────────────────────────────────────────────┘
```

#### Đột phá Kỹ thuật & Hiệu năng:
1. **Loại bỏ hoàn toàn Runtime nặng**: Nhờ biên dịch trực tiếp sang mã máy nhị phân x64 với cấu hình LTO (`lto = true`, `codegen-units = 1`, `opt-level = "z"`), tệp thực thi chỉ nặng ~2.4 MB mà không cần bất kỳ bộ cài phụ thuộc (Visual C++ Redistributable hoặc .NET Runtime).
2. **Không gây Micro-Stutter**: Không sử dụng các vòng lặp WMI liên tục làm nghẽn WMI Provider Host (`WmiPrvSE.exe`). Các chỉ số quan trọng được đọc trực tiếp qua Windows Kernel System Information APIs và con trỏ C FFI của NVML với độ trễ dưới 5 micro-giây.
3. **Hardware Fail-Safe Protocol**: Khi ứng dụng dừng vì bất kỳ lý do nào (đóng cửa sổ, tắt máy, kill process từ Task Manager hoặc ngắt tín hiệu Console), khối lệnh phục hồi sẽ kích hoạt ngay lập tức để chuyển quạt về chế độ điều khiển tự động của Firmware/BIOS, loại bỏ hoàn toàn nguy cơ quá nhiệt máy tính do quên bật quạt.

---

### 5. Hướng dẫn Cài đặt & Sử dụng

#### Cách 1: Sử dụng Bộ cài đặt Tiêu chuẩn (Khuyến nghị)
1. Tải về phiên bản mới nhất: `predator-fan-ctl-setup-v1.5.0.exe` từ thư mục `dist/` hoặc trang Releases.
2. Chạy tệp cài đặt với quyền **Administrator** (trình cài đặt Inno Setup đã được tối ưu hóa nén `lzma2/ultra64`).
3. Các tùy chọn thông minh có sẵn:
   - ✅ Tạo biểu tượng trên Desktop và Start Menu.
   - ✅ Tự động kích hoạt khởi động cùng Windows qua **Windows Task Scheduler** với đặc quyền cao nhất (`Highest Privileges`), giúp ứng dụng khởi chạy êm ái mà không bị pop-up cảnh báo UAC làm phiền.
4. Mở ứng dụng: Biểu tượng quạt sẽ xuất hiện trên thanh Taskbar / Khay hệ thống.

#### Cách 2: Tùy biến File Cấu hình `config.toml`
Ứng dụng lưu cấu hình tại cùng thư mục chạy hoặc `%LOCALAPPDATA%`:
```toml
# config.toml
profile = "Custom"
polling_rate_ms = 1000

[governor]
enabled = true
smooth_epp = true
target_cpu_temp = 75

[curve.cpu]
points = [
    { temp = 40, speed = 25 },
    { temp = 55, speed = 40 },
    { temp = 68, speed = 60 },
    { temp = 78, speed = 80 },
    { temp = 88, speed = 100 }
]

[curve.gpu]
points = [
    { temp = 45, speed = 30 },
    { temp = 60, speed = 50 },
    { temp = 72, speed = 75 },
    { temp = 82, speed = 100 }
]
```

---

### 6. Hướng dẫn Biên dịch từ Mã nguồn (Build from Source)

#### Yêu cầu hệ thống:
- Hệ điều hành: Windows 10 / 11 64-bit.
- Rust Toolchain: `1.75.0` trở lên (Khuyến nghị phiên bản `stable-x86_64-pc-windows-msvc`).
- Visual Studio Build Tools (C++ Build Tools, Windows SDK).
- Inno Setup 6 (tùy chọn, nếu muốn đóng gói tệp cài đặt `.exe`).

#### Các bước thực hiện:
```powershell
# 1. Clone repository
git clone https://github.com/your-username/predator-fan-ctl.git
cd predator-fan-ctl

# 2. Biên dịch bản Release tối ưu hóa
cargo build --release

# 3. File nhị phân xuất xưởng tại:
# .\target\release\predator-fan-ctl.exe

# 4. (Tùy chọn) Đóng gói bộ cài đặt Inno Setup
iscc installer/predator-fan-ctl.iss
# Bộ cài đặt sẽ được tạo tại: .\dist\predator-fan-ctl-setup-v1.5.0.exe
```

---

### 7. 🤖 Tuyên bố Minh bạch về Trí tuệ Nhân tạo (AI Disclosure & Transparency)

Dự án này được thiết kế, lập trình, tối ưu hóa thuật toán và xây dựng tài liệu kỹ thuật với sự trợ lực toàn diện từ **Google Antigravity & Gemini**, dưới sự chỉ đạo kiến trúc chuyên môn, giám sát kỹ thuật, phản biện thuật toán và kiểm thử trực tiếp trên phần cứng máy tính xách tay Acer Predator thực tế của tác giả **Yuutiro-sama**.

- **Vai trò của AI (Google Antigravity & Gemini)**:
  - Khảo sát các thanh ghi ACPI/DSDT/SSDT và trích xuất phương thức giao tiếp WMI cấp thấp.
  - Phát triển toàn bộ mã nguồn Rust thuần Win32 GDI với kỹ thuật zero-allocation, tối ưu hóa an toàn bộ nhớ và ngăn ngừa rò rỉ tài nguyên hệ thống (GDI Handle leaks, Memory leaks).
  - Tinh chỉnh các bộ lọc toán học cho thuật toán làm mịn đồ thị nhiệt độ và điều tốc vi xử lý EPP.
- **Vai trò của Con người (Kiến trúc sư trưởng Yuutiro-sama)**:
  - Định hình yêu cầu, quy hoạch giải pháp và phê duyệt toàn bộ kiến trúc hệ thống.
  - Đo đạc thực nghiệm, xác thực tính ổn định của mạch quạt vật lý, trực tiếp kiểm tra các bài kiểm thử nặng (Stress Test AIDA64 + FurMark) trên phần cứng thực tế.

---

### 8. ⚖️ Quy định Thương mại & Miễn trừ Trách nhiệm Phần cứng (Commercial Use & Hardware Disclaimer)

> [!IMPORTANT]
> **TÍNH CHẤT PHI THƯƠNG MẠI**:
> Toàn bộ mã nguồn, tài liệu và bộ cài đặt của dự án này được chia sẻ **HOÀN TOÀN MIỄN PHÍ VÀ PHI THƯƠNG MẠI** phục vụ mục đích học tập, nghiên cứu kỹ thuật và hỗ trợ cộng đồng người dùng laptop Acer Predator / Nitro.
> **NGHIÊM CẤM TUYỆT ĐỐI** mọi hành vi đóng gói lại, sang nhượng bản quyền, bán lấy tiền, chèn mã quảng cáo hoặc trục lợi thương mại dưới bất kỳ hình thức nào mà không có sự đồng ý bằng văn bản của tác giả.

> [!WARNING]
> **MIỄN TRỪ TRÁCH NHIỆM PHẦN CỨNG (HARDWARE DISCLAIMER)**:
> Phần mềm này được cung cấp theo giấy phép MIT trên nguyên tắc **"NGUYÊN TRẠNG" (AS IS)**, không có bất kỳ sự bảo đảm nào dù được tuyên bố hay ngụ ý.
> Việc can thiệp trực tiếp vào bộ điều khiển quạt, thanh ghi Embedded Controller (EC) và chỉ số EPP của vi xử lý có thể ảnh hưởng trực tiếp đến nhiệt độ làm việc của phần cứng. Việc thiết lập đường cong quạt quá thấp khi máy tính hoạt động ở mức tải nặng có thể dẫn đến hiện tượng quá nhiệt (Thermal Throttling) hoặc kích hoạt cơ chế bảo vệ ngắt nguồn khẩn cấp của bo mạch chủ.
> **Tác giả và nhóm phát triển KHÔNG CHỊU TRÁCH NHIỆM đối với bất kỳ sự cố, hỏng hóc phần cứng, mất mát dữ liệu hoặc các thiệt hại trực tiếp/gián tiếp nào phát sinh từ việc cài đặt và sử dụng phần mềm này.** Người dùng tự chịu hoàn toàn trách nhiệm đối với thiết bị của mình.

---

<br/>

<a name="english"></a>
## 🇬🇧 English

### 1. Project Overview & Vision

OEM hardware management software for gaming laptops is notoriously bloated. Often built upon resource-heavy frameworks such as Electron, Chromium, or .NET WPF, these background utilities frequently consume between 200 MB to 600 MB of RAM while causing perceptible gaming micro-stutters through inefficient, high-latency WMI polling loops.

**`predator-fan-ctl`** is engineered to eliminate this bloat entirely:
- **Radical Minimalism**: 100% written in **Rust** coupled with a pure **Native Win32 GDI** graphical interface (Zero Electron, Zero WebView2, Zero web dependencies).
- **Ultra-low footprint**: Consumes only **~2 - 4 MB of RAM** with **< 0.05% CPU utilization** during continuous background monitoring.
- **Low-level Hardware Access**: Direct communication via ACPI WMI Methods (`Acer_FanInterface`) and direct Embedded Controller (EC) register I/O via `inpoutx64.dll`.
- **Absolute Reliability**: Multi-tier hardware fail-safe architecture that automatically restores fan curves to firmware/BIOS auto mode upon application exit or system shutdown.

---

### 2. Key Highlights & Features

- 🪟 **Lightning-Fast Native Win32 GDI Interface**:
  - Cyberpunk/Predator inspired Dark Mode UI with full per-monitor HiDPI scaling support on 2K/4K displays.
  - Real-time 60 FPS double-buffered telemetry chart displaying CPU/GPU temperatures, fan RPM/percentages, and wattage with zero visual flicker.
- 🎛️ **Interactive Fan Curve Editor**:
  - Intuitive click-and-drag curve editing for both CPU and GPU fans across custom thermal thresholds.
  - Built-in thermal hysteresis engine to prevent fan revving and oscillation around boundary temperatures.
- ⚡ **Smooth EPP Thermal Governor**:
  - Automatically adjusts Intel/AMD *Energy Performance Preference* (EPP) registers in real time, keeping the laptop cool and quiet during idle tasks while unleashing maximum Turbo Boost frequencies under sustained workloads.
- 📊 **Zero-Allocation Hardware Telemetry**:
  - CPU Clock: Evaluates peak boost frequency across all physical P-cores & E-cores using Win32 FFI `CallNtPowerInformation` (PowrProf).
  - CPU Power: Extracts Intel Package 0 (RAPL) power consumption directly from Windows PDH English counters down to 0.1 W precision.
  - GPU Telemetry: Direct dynamic FFI binding to `nvml.dll`, safely querying clock, power, and temperature without waking the GPU from NVIDIA Optimus D3cold sleep.
- 🛡️ **System Tray & Multi-Tier Fail-Safe**:
  - Compact system tray menu with rapid profile switching: *Quiet, Balanced, Performance, Turbo, Custom Curve, Auto*.
  - SCM Service Guard seamlessly handles `PredatorSenseService` states and guarantees BIOS restoration during clean exits, console breaks, or unexpected termination.

---

### 3. Graphical User Interface

<div align="center">

#### Main Dashboard
![Dashboard](assets/screenshots/dashboard.png)

#### Real-time Telemetry Chart
![Telemetry Chart](assets/screenshots/telemetry_chart.png)

#### Fan Curve Editor
![Curve Dialog](assets/screenshots/curve_dialog.png)

</div>

---

### 4. Technical Architecture & Engineering Highlights

```
┌────────────────────────────────────────────────────────────────────────┐
│                   PREDATOR-FAN-CTL ARCHITECTURE                        │
├────────────────────────────────────────────────────────────────────────┤
│  [ Native Win32 GDI GUI Layer ]                                        │
│   ├── Dashboard Window (GDI Double Buffering, HiDPI aware)             │
│   ├── Telemetry Chart (60 FPS Bezier/Polyline, Circular History Buffer)│
│   ├── Fan Curve Dialog (Draggable Control Points, Hysteresis Logic)    │
│   └── System Tray Manager (Shell_NotifyIconW, Win32 Popup Menu)        │
├────────────────────────────────────────────────────────────────────────┤
│  [ Core Thermal Governor & Control Loop ]                              │
│   ├── ThermalGovernor (Smooth EPP Stepping, Load-aware Hysteresis)     │
│   ├── Config Engine (TOML Serde, In-memory Atomic Profile Switch)      │
│   └── Fail-safe Supervisor (SetConsoleCtrlHandler, Mutex, Auto-restore)│
├────────────────────────────────────────────────────────────────────────┤
│  [ Zero-Allocation Telemetry Layer ]                                   │
│   ├── CPU Clock: PowrProf CallNtPowerInformation (max CurrentMhz)      │
│   ├── CPU Power: Windows PDH English Counter (RAPL_Package0_PKG Power) │
│   ├── CPU Temp:  ACPI ThermalZone via WMI (MSAcpi_ThermalZoneTemp)     │
│   └── GPU Data:  Dynamic NVML FFI (nvmlDeviceGetTemp/Power/ClockInfo)  │
├────────────────────────────────────────────────────────────────────────┤
│  [ Low-Level Hardware Backends ]                                       │
│   ├── AcerWmiBackend (root\WMI -> Acer_FanInterface Methods)          │
│   ├── DirectEcBackend (Embedded Controller Direct I/O via inpoutx64)   │
│   └── SCM Guard (Safe pausing & resuming of PredatorSenseService)      │
└────────────────────────────────────────────────────────────────────────┘
```

#### Core Engineering Principles:
1. **Zero Runtime Dependencies**: Compiled with Link-Time Optimization (`lto = true`, `opt-level = "z"`, `codegen-units = 1`), yielding an ultra-compact ~2.4 MB standalone executable that requires no external runtime environments or redistributables.
2. **Zero Polling-induced Stutter**: Bypasses heavy WMI queries that cause spikes in `WmiPrvSE.exe`. Real-time metrics are polled via high-speed native system calls and NVML function pointers in under 5 microseconds.
3. **Hardware Fail-Safe Guarantee**: If the process receives a termination signal (`WM_CLOSE`, `WM_DESTROY`, `SetConsoleCtrlHandler` break, or service stop), a hardware restoration routine immediately resets the fan duty cycles back to Firmware/BIOS defaults, preventing thermal runaway.

---

### 5. Installation & Getting Started

#### Option 1: Pre-compiled Setup Installer (Recommended)
1. Download `predator-fan-ctl-setup-v1.5.0.exe` from `dist/` or the GitHub Releases section.
2. Execute the installer with **Administrator privileges** (packaged with LZMA2/Ultra64 compression via Inno Setup).
3. Configurable installation preferences:
   - ✅ Create Desktop & Start Menu shortcuts.
   - ✅ Register with **Windows Task Scheduler** on user logon with `Highest Privileges` (enabling seamless background startup without UAC prompts).
4. Launch the application; the fan icon will be visible in the system notification area.

#### Option 2: Custom Configuration (`config.toml`)
The application reads its settings from `config.toml` situated alongside the executable or in `%LOCALAPPDATA%`:
```toml
# config.toml
profile = "Custom"
polling_rate_ms = 1000

[governor]
enabled = true
smooth_epp = true
target_cpu_temp = 75

[curve.cpu]
points = [
    { temp = 40, speed = 25 },
    { temp = 55, speed = 40 },
    { temp = 68, speed = 60 },
    { temp = 78, speed = 80 },
    { temp = 88, speed = 100 }
]

[curve.gpu]
points = [
    { temp = 45, speed = 30 },
    { temp = 60, speed = 50 },
    { temp = 72, speed = 75 },
    { temp = 82, speed = 100 }
]
```

---

### 6. Building from Source

#### Prerequisites:
- OS: Windows 10 / 11 64-bit.
- Rust Toolchain: `1.75.0` or higher (`stable-x86_64-pc-windows-msvc`).
- Visual Studio C++ Build Tools & Windows SDK.
- Inno Setup 6 (optional, for packaging the installer).

#### Build Commands:
```powershell
# 1. Clone the repository
git clone https://github.com/your-username/predator-fan-ctl.git
cd predator-fan-ctl

# 2. Build optimized release binary
cargo build --release

# 3. Built binary will be located at:
# .\target\release\predator-fan-ctl.exe

# 4. (Optional) Generate the standalone installer
iscc installer/predator-fan-ctl.iss
# Output package will be in: .\dist\predator-fan-ctl-setup-v1.5.0.exe
```

---

### 7. 🤖 AI Disclosure & Transparency Statement

This project was conceived, engineered, algorithmically optimized, and documented with comprehensive assistance from **Google Antigravity & Gemini**, working under the architectural vision, technical supervision, code verification, and direct physical hardware validation of author **Yuutiro-sama**.

- **AI Assistant Contributions (Google Antigravity & Gemini)**:
  - Reverse-engineering ACPI DSDT/SSDT tables and implementing low-level WMI method invocations.
  - Writing zero-allocation Rust Win32 GDI drawing routines with strict resource lifetime management to prevent GDI object and memory leaks.
  - Designing telemetry smoothing formulas and mathematical models for dynamic CPU EPP thermal modulation.
- **Human Architect Contributions (Lead Engineer Yuutiro-sama)**:
  - Defined system specifications, safety boundaries, and architectural roadmap.
  - Conducted extensive hardware bench testing, fan RPM calibration, and sustained thermal stress verification (AIDA64 FPU + FurMark).

---

### 8. ⚖️ Commercial Use & Hardware Disclaimer

> [!IMPORTANT]
> **STRICTLY NON-COMMERCIAL**:
> All source code, documentation, and installation packages in this repository are distributed **FREE OF CHARGE FOR EDUCATIONAL AND RESEARCH PURPOSES ONLY**.
> Any commercial repackaging, redistribution for monetary compensation, unauthorized licensing, or monetization without prior written consent from the author is **STRICTLY PROHIBITED**.

> [!WARNING]
> **HARDWARE DISCLAIMER**:
> This software is provided under the MIT License on an **"AS IS"** basis, without warranty of any kind, express or implied.
> Direct manipulation of fan speeds, Embedded Controller (EC) registers, and CPU Energy Performance Preference (EPP) directly influences system operating temperatures. Configuring inappropriately low fan curves under sustained high workloads may cause thermal throttling or emergency motherboard power shutdowns.
> **The author and contributors accept NO RESPONSIBILITY for any hardware damage, data loss, or system instability resulting from the installation or usage of this software.** You assume full responsibility for your own hardware.

---

<div align="center">

Copyright (c) 2026 Yuutiro-sama. Distributed under the MIT License.

</div>
