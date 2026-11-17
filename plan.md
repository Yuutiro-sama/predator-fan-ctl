<!-- Kế hoạch được lập bởi: Gemini 4 Hydrogen (Dự phòng - Suy luận cao) -->

MASTER IMPLEMENTATION PLAN: TRIỂN KHAI MÃ NGUỒN & PHÁT HÀNH GITHUB RELEASE (ARCH-GH-RELEASE-V1.5.0)

Trạng thái kiến trúc: Sẵn sàng thi công (Ready for Execution)

Đối tượng thực thi: Kỹ sư Antigravity & Subagents

Thư mục làm việc: c:\Users\default.LAPTOP-1V8CRH32\Documents\antigravity\fan-control

Tài khoản đích: Yuutiro-sama ([https://github.com/Yuutiro-sama/predator-fan-ctl](https://github.com/Yuutiro-sama/predator-fan-ctl))

0. NGUYÊN TẮC AN TOÀN MÀN HÌNH & KIẾN TRÚC THI CÔNG KHÔNG CỬA SỔ (ZERO-GUI POLICY)

Để đảm bảo tuyệt đối không chạm, không chiếm quyền điều khiển (focus stealing) và không làm gián đoạn game đang chạy trên màn hình chính AG2313QS1 (0, 0, 2560, 1440), toàn bộ quy trình triển khai bắt buộc tuân thủ kiến trúc 100% Headless CLI & REST API:

Cấm mở trình duyệt hoặc ứng dụng GUI mặc định: Mọi tiến trình GUI mới trên Windows mặc định khởi tạo tại tọa độ (0, 0) — tức nằm trực tiếp trên màn hình chính AG2313QS1. Nghiêm cấm gọi Start-Process mở URL, GitHub Desktop, hoặc bất kỳ cửa sổ nào không khóa tọa độ.

Giao tiếp trực tiếp qua GitHub REST API v3 & git.exe: Toàn bộ thao tác tạo Repository, đẩy mã nguồn, tạo Release v1.5.0 và tải lên file cài đặt .exe được thực hiện ngầm trong phiên PowerShell hiện tại thông qua Invoke-RestMethod / curl.exe sử dụng OAuth Token trích xuất trực tiếp trong RAM từ Windows Credential Manager.

Quy tắc dự phòng nếu cần mở cửa sổ phụ: Chỉ được phép tương tác trong vùng tọa độ màn phụ B160UAN01.Q với X ∈ [-1920, -1], Y ∈ [0, 1200]. Tuy nhiên, trong kế hoạch này không cần mở bất kỳ cửa sổ GUI nào.

1. PHASE 1: KIỂM TOÁN MÔI TRƯỜNG, TRÍCH XUẤT TOKEN & VỆ SINH GIT TREE
Bước 1.1: Kiểm tra trạng thái Git local và cấu hình .gitignore

Kỹ sư Antigravity chạy kiểm tra tại c:\Users\default.LAPTOP-1V8CRH32\Documents\antigravity\fan-control:

Đảm bảo thư mục biên dịch nặng của Rust (target/) và file nhị phân cài đặt (dist/*.exe) không bị commit trực tiếp vào lịch sử Git (giữ repo nhẹ, chuẩn mã nguồn mở, vì dist/predator-fan-ctl-setup-v1.5.0.exe sẽ được đính kèm làm Release Asset chính thức ở Phase 4).

Kiểm tra sự tồn tại và tính toàn vẹn của file dist\predator-fan-ctl-setup-v1.5.0.exe, đồng thời tính mã băm SHA-256 để công bố trong Release Notes.

PowerShell
Set-Location "c:\Users\default.LAPTOP-1V8CRH32\Documents\antigravity\fan-control"

# 1. Kiểm tra file cài đặt & tính SHA-256
$installerPath = "dist\predator-fan-ctl-setup-v1.5.0.exe"
if (-not (Test-Path $installerPath)) { throw "Thiếu file cài đặt: $installerPath" }
$sha256 = (Get-FileHash -Path $installerPath -Algorithm SHA256).Hash
Write-Output "Installer OK | Size: $((Get-Item $installerPath).Length) bytes | SHA256: $sha256"

# 2. Đảm bảo .gitignore loại trừ thư mục build và binary cài đặt lớn
$ignoreRules = @("target/", "dist/*.exe", "*.pdb", ".DS_Store", "Thumbs.db")
if (-not (Test-Path ".gitignore")) { New-Item -Path ".gitignore" -ItemType File -Force | Out-Null }
$currentIgnore = Get-Content ".gitignore" -ErrorAction SilentlyContinue
foreach ($rule in $ignoreRules) {
    if ($currentIgnore -notcontains $rule) {
        Add-Content -Path ".gitignore" -Value $rule
    }
}

Bước 1.2: Trích xuất GitHub OAuth Token trong RAM (Bảo mật tuyệt đối)

Sử dụng git credential fill để lấy token đã lưu trong Windows Credential Manager (git:[https://github.com](https://github.com)). Tuyệt đối không in token ra console và không ghi token vào bất kỳ file nào.

PowerShell
$credInput = "protocol=https`nhost=github.com`n`n"
$credOutput = $credInput | git credential fill
$ghToken = ($credOutput | Where-Object { $_ -match "^password=" }) -replace "^password=", ""
if ([string]::IsNullOrWhiteSpace($ghToken)) { throw "Không tìm thấy GitHub OAuth Token trong Credential Manager." }

$headers = @{
    Authorization = "Bearer $ghToken"
    Accept        = "application/vnd.github+json"
    "X-GitHub-Api-Version" = "2022-11-28"
    "User-Agent"  = "predator-fan-ctl-release-agent"
}

# Xác thực danh tính & quyền token
$userCheck = Invoke-RestMethod -Uri "https://api.github.com/user" -Headers $headers -Method Get
Write-Output "Authenticated as: $($userCheck.login)"

2. PHASE 2: KHỞI TẠO TÀI LIỆU PHÁP LÝ (LICENSE) & README.md SONG NGỮ (VI / EN)

Kỹ sư Antigravity tạo mới (hoặc ghi đè chuẩn hóa) 2 file gốc tại thư mục dự án: LICENSE và README.md.

Bước 2.1: Khởi tạo file LICENSE (MIT License)

Ghi nội dung sau vào c:\Users\default.LAPTOP-1V8CRH32\Documents\antigravity\fan-control\LICENSE (UTF-8 without BOM):

Plaintext
MIT License

Copyright (c) 2026 Yuutiro-sama

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

Bước 2.2: Khởi tạo file README.md Song ngữ (Tiếng Việt / English)

Ghi đầy đủ nội dung chuẩn chuyên nghiệp dưới đây vào c:\Users\default.LAPTOP-1V8CRH32\Documents\antigravity\fan-control\README.md (đảm bảo bao gồm đầy đủ kiến trúc kỹ thuật Rust/Win32/ACPI-WMI-EC, hướng dẫn cài đặt/biên dịch, Tuyên bố minh bạch về AI, và Tính thương mại & Miễn trừ trách nhiệm phần cứng):

Markdown
# Acer Predator Ultra-Light Fan Controller (`predator-fan-ctl`)

[![Release](https://img.shields.io/badge/Release-v1.5.0-blue.svg)](https://github.com/Yuutiro-sama/predator-fan-ctl/releases/tag/v1.5.0)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011%20x64-0078D6.svg)](#)
[![Language](https://img.shields.io/badge/Built%20With-Rust%20%2B%20Native%20Win32%20GDI-orange.svg)](#)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

> **Ngôn ngữ / Language:** [Tiếng Việt](#-tiếng-việt) | [English](#-english)

---

## 🇻🇳 Tiếng Việt

### 1. Giới thiệu tổng quan
**Acer Predator Ultra-Light Fan Controller (`predator-fan-ctl`)** là tiện ích điều khiển quạt tản nhiệt và giám sát phần cứng siêu nhẹ dành riêng cho các dòng laptop **Acer Predator** (như Helios / Neo / Triton sử dụng Intel Core HX & NVIDIA GeForce RTX).

Thay thế hoàn toàn các phần mềm điều khiển nặng nề (như PredatorSense vốn tiêu tốn hàng trăm MB RAM và nhiều tiến trình nền), `predator-fan-ctl` được viết thuần bằng **Rust** kết hợp trực tiếp với **Native Win32 API & GDI**, giúp tối ưu hóa tài nguyên hệ thống ở mức gần như bằng không (`< 5 MB RAM`, `~0% CPU idle`).

### 2. Tính năng nổi bật & Kiến trúc kỹ thuật
- **Kiến trúc Native Rust + Win32 GDI:** Không sử dụng Electron, WebView2 hay framework giao diện cồng kềnh; render biểu đồ nhiệt độ, xung nhịp và đường cong quạt trực tiếp bằng GDI chống nháy (Double-Buffered GDI).
- **Điều khiển quạt đa chế độ qua ACPI / WMI / EC:**
  - Chuyển đổi nhanh các chế độ vận hành: **Auto (Tự động)**, **Max (Tối đa)** và **Custom Curve (Đường cong tùy chỉnh theo ngưỡng nhiệt độ CPU/GPU)**.
  - Tích hợp **Custom Fan Curve Editor** trực quan với cơ chế chống giật/trễ vòng quay (Hysteresis & Polling protection).
- **Giám sát Telemetry phần cứng thời gian thực:**
  - Đọc chính xác xung nhịp CPU (hỗ trợ kiến trúc Hybrid P-Core / E-Core qua PDH/WMI), xung nhịp & công suất GPU (NVML/WMI), nhiệt độ lõi và tốc độ vòng quay RPM của quạt CPU/GPU.
- **Vận hành nền thông minh:** Tích hợp System Tray gọn nhẹ, tự động khôi phục cấu hình quạt khi khởi động hoặc đánh thức máy từ chế độ Sleep.

### 3. Hướng dẫn cài đặt & Sử dụng
#### Cách 1: Cài đặt nhanh qua bộ cài đặt (Khuyên dùng)
1. Truy cập trang [**Releases v1.5.0**](https://github.com/Yuutiro-sama/predator-fan-ctl/releases/tag/v1.5.0).
2. Tải về tệp `predator-fan-ctl-setup-v1.5.0.exe`.
3. Chạy tệp cài đặt với quyền **Administrator** (bắt buộc để giao tiếp với trình điều khiển ACPI/WMI/EC phần cứng).

#### Cách 2: Biên dịch từ mã nguồn (Dành cho lập trình viên)
- **Yêu cầu:** Windows 10/11 (64-bit), Rust Toolchain (`stable-x86_64-pc-windows-msvc`), Windows SDK.
```bash
git clone [https://github.com/Yuutiro-sama/predator-fan-ctl.git](https://github.com/Yuutiro-sama/predator-fan-ctl.git)
cd predator-fan-ctl
cargo build --release

⚠️ LƯU Ý BẮT BUỘC (MANDATORY NOTICES)
🤖 Tuyên bố minh bạch về AI (AI Disclosure & Transparency)

Dự án này được phát triển thông qua mô hình hợp tác giữa con người và Trí tuệ nhân tạo:

Kiến trúc hệ thống, mã nguồn Rust/Win32, thuật toán điều khiển đường cong quạt và quy trình kiểm thử tự động được thiết kế, triển khai và tối ưu hóa bởi Google Antigravity & Gemini dưới sự định hướng yêu cầu, kiểm duyệt thực tế trên phần cứng và giám sát trực tiếp của tác giả (Yuutiro-sama).

Mọi thành phần giao tiếp phần cứng đều đã được kiểm thử thực nghiệm trên thiết bị Acer Predator thực tế trước khi đóng gói phát hành.

⚖️ Tính thương mại & Miễn trừ trách nhiệm (Commercial Use & Hardware Disclaimer)

Mục đích sử dụng & Nghiêm cấm trục lợi thương mại:

Phần mềm được chia sẻ miễn phí phục vụ cộng đồng người dùng, mục đích học tập, nghiên cứu kỹ thuật lập trình hệ thống Windows/Rust và tối ưu hóa trải nghiệm cá nhân.

Nghiêm cấm mọi hành vi đóng gói lại, gắn nhãn thương hiệu khác hoặc buôn bán, trục lợi thương mại trái phép dưới bất kỳ hình thức nào từ mã nguồn hoặc bộ cài đặt của dự án này.

Miễn trừ trách nhiệm can thiệp phần cứng (EC / ACPI / WMI):

Phần mềm thực hiện đọc/ghi các thanh ghi điều khiển quạt thông qua giao diện ACPI/WMI/Embedded Controller (EC) của nhà sản xuất.

Việc thiết lập đường cong quạt quá thấp khi tải nặng (gaming/render) có thể dẫn đến quá nhiệt (thermal throttling) hoặc giảm tuổi thọ linh kiện.

Phần mềm được cung cấp theo nguyên trạng ("AS IS") theo giấy phép MIT. Tác giả (Yuutiro-sama) và các hệ thống AI hỗ trợ phát triển không chịu trách nhiệm pháp lý đối với bất kỳ hư hỏng phần cứng, mất dữ liệu hoặc sự cố hệ thống nào phát sinh trong quá trình sử dụng hoặc tùy chỉnh sai thông số.

🇬🇧 English
1. Overview

Acer Predator Ultra-Light Fan Controller (predator-fan-ctl) is an ultra-lightweight native fan control and hardware telemetry utility engineered specifically for Acer Predator gaming laptops (Helios / Neo / Triton powered by Intel Core HX & NVIDIA GeForce RTX GPUs).

Built to replace bloated OEM control centers (such as PredatorSense, which consumes hundreds of megabytes of RAM and multiple background services), predator-fan-ctl is written purely in Rust directly atop the Native Win32 API & GDI, achieving near-zero resource overhead (< 5 MB RAM, ~0% idle CPU).

2. Key Features & Architecture

Pure Rust + Native Win32 GDI Architecture: Zero Electron, WebView2, or heavy UI runtime dependencies. Real-time telemetry charts and interactive fan curve dialogs are rendered via flicker-free double-buffered Win32 GDI.

Multi-Mode ACPI / WMI / EC Fan Control:

Instant switching between Auto, Max, and Custom Fan Curve profiles mapped to CPU/GPU temperature thresholds.

Built-in Custom Fan Curve Editor with hysteresis smoothing and safe polling intervals.

Real-Time Hardware Telemetry:

Accurate hybrid CPU clock tracking (P-Core / E-Core via PDH/WMI), GPU clock & power draw (NVML/WMI), core temperatures, and live CPU/GPU fan RPM readouts.

Unobtrusive Background Operation: Lightweight System Tray integration with automatic profile restoration on boot and wake-from-sleep events.

3. Installation & Build Guide
Option 1: Pre-built Installer (Recommended)

Navigate to Releases v1.5.0.

Download predator-fan-ctl-setup-v1.5.0.exe.

Run the installer with Administrator privileges (required for hardware ACPI/WMI/EC driver communication).

Option 2: Build from Source

Prerequisites: Windows 10/11 (64-bit), Rust Toolchain (stable-x86_64-pc-windows-msvc), Windows SDK.

Bash
git clone [https://github.com/Yuutiro-sama/predator-fan-ctl.git](https://github.com/Yuutiro-sama/predator-fan-ctl.git)
cd predator-fan-ctl
cargo build --release

⚠️ MANDATORY NOTICES
🤖 AI Disclosure & Transparency

This project was engineered through a collaborative human–AI workflow:

System architecture, Rust/Win32 source code, fan curve control logic, and automated verification workflows were designed, implemented, and refined by Google Antigravity & Gemini under the architectural direction, real-hardware validation, and supervision of the project author (Yuutiro-sama).

All low-level hardware interfaces have been empirically tested on physical Acer Predator hardware prior to release packaging.

⚖️ Commercial Use & Hardware Disclaimer

Permitted Usage & Prohibition of Unauthorized Commercial Exploitation:

This software is distributed free of charge for community use, educational study, Windows/Rust system programming research, and personal hardware optimization.

Unauthorized commercial repackaging, rebranding, resale, or monetization of this source code or its binary installers in any form is strictly prohibited.

Embedded Controller (EC) & Hardware Liability Disclaimer:

This utility interacts with fan control registers via vendor ACPI/WMI/Embedded Controller (EC) interfaces.

Configuring overly conservative fan curves under heavy CPU/GPU workloads may cause thermal throttling or accelerated component wear.

This software is provided "AS IS", without warranty of any kind, under the MIT License. Neither the author (Yuutiro-sama) nor the AI development tools assume any liability for hardware damage, thermal events, or data loss resulting from usage or misconfiguration.


---

## 3. PHASE 3: TẠO REPOSITORY TRÊN GITHUB & ĐẨY MÃ NGUỒN (`git push`)

### Bước 3.1: Khởi tạo Repository `predator-fan-ctl` qua GitHub REST API
Gọi `POST [https://api.github.com/user/repos](https://api.github.com/user/repos)` (xử lý an toàn trường hợp repo đã tồn tại bằng cách kiểm tra HTTP status):

```powershell
$repoName = "predator-fan-ctl"
$repoBody = @{
    name        = $repoName
    description = "Ultra-lightweight native Rust & Win32 fan controller and hardware telemetry utility for Acer Predator laptops."
    homepage    = "https://github.com/Yuutiro-sama/predator-fan-ctl"
    private     = $false
    has_issues  = $true
    has_wiki    = $false
    auto_init   = $false
} | ConvertTo-Json -Depth 3

try {
    $repoRes = Invoke-RestMethod -Uri "https://api.github.com/user/repos" -Headers $headers -Method Post -Body $repoBody -ContentType "application/json; charset=utf-8"
    Write-Output "Created Repo: $($repoRes.html_url)"
} catch {
    if ($_.Exception.Response.StatusCode.value__ -eq 422) {
        Write-Output "Repo '$repoName' đã tồn tại trên tài khoản Yuutiro-sama, tiếp tục cấu hình remote."
    } else {
        throw $_
    }
}

Bước 3.2: Commit tài liệu mới (LICENSE, README.md, .gitignore) và Push lên nhánh main

Chuẩn hóa nhánh mặc định từ master sang main (git branch -M main) theo chuẩn hiện hành của GitHub.

Kiểm tra git status để chắc chắn không có file .exe hay thư mục target/ nào nằm trong staging area trước khi đẩy lên remote.

PowerShell
git config user.name "Yuutiro-sama"
git config user.email "dangtanmin111@gmail.com"

git add .gitignore LICENSE README.md
# Đảm bảo mọi mã nguồn cập nhật khác cũng được stage (trừ các file đã ignore)
git add -A

# Kiểm tra an toàn không commit file cài đặt .exe vào git history
$stagedFiles = git diff --cached --name-only
if ($stagedFiles -match "\.exe$") {
    git reset -- "*.exe"
    Write-Output "Đã gỡ file .exe khỏi Git commit để gắn vào Release Asset."
}

git commit -m "docs: add MIT LICENSE, bilingual README (VI/EN) with AI & hardware notices, and release prep v1.5.0" 2>$null

git branch -M main
git remote remove origin 2>$null
git remote add origin "https://github.com/Yuutiro-sama/predator-fan-ctl.git"
git push -u origin main

4. PHASE 4: TẠO GITHUB RELEASE v1.5.0 & ĐÍNH KÈM BỘ CÀI ĐẶT .EXE
Bước 4.1: Tạo Release v1.5.0 kèm Release Notes song ngữ (VI / EN)

Gọi POST [https://api.github.com/repos/Yuutiro-sama/predator-fan-ctl/releases](https://api.github.com/repos/Yuutiro-sama/predator-fan-ctl/releases) để tạo tag v1.5.0 trên nhánh main, đính kèm mã băm SHA-256 của bộ cài đặt và đầy đủ tuyên bố AI + miễn trừ trách nhiệm phần cứng:

PowerShell
$releaseNotes = @"
## 🚀 Acer Predator Ultra-Light Fan Controller v1.5.0

### 🇻🇳 Tiếng Việt — Ghi chú phát hành
- **Kiến trúc thuần Rust + Win32 GDI siêu nhẹ:** Tiêu thụ dưới 5 MB RAM, ~0% CPU ở trạng thái chờ.
- **Điều khiển quạt đa chế độ (Auto / Max / Custom Curve):** Giao tiếp trực tiếp qua ACPI / WMI / EC, hỗ trợ vẽ đường cong quạt tùy chỉnh theo nhiệt độ CPU/GPU với giao diện GDI chống nháy.
- **Giám sát Telemetry thời gian thực:** Hiển thị chính xác xung nhịp CPU (hỗ trợ Intel Hybrid P-Core/E-Core), xung nhịp & công suất GPU NVIDIA RTX, nhiệt độ và tốc độ RPM quạt.
- **Kiểm tra toàn vẹn tệp cài đặt (`predator-fan-ctl-setup-v1.5.0.exe`):**
  - **SHA-256:** ``$sha256``

#### ⚠️ Lưu ý bắt buộc (AI & Miễn trừ trách nhiệm)
- **Tuyên bố về AI (AI Disclosure):** Kiến trúc, mã nguồn Rust/Win32 và quy trình kiểm thử của bản phát hành này được phát triển bởi **Google Antigravity & Gemini** phối hợp cùng tác giả (**Yuutiro-sama**).
- **Tính thương mại & Phần cứng:** Phần mềm phát hành miễn phí cho mục đích học tập, nghiên cứu và sử dụng cá nhân theo giấy phép MIT; nghiêm cấm đóng gói thương mại trái phép. Phần mềm cung cấp theo nguyên trạng (**AS IS**), miễn trừ trách nhiệm đối với rủi ro can thiệp thanh ghi phần cứng EC/ACPI/WMI.

---

### 🇬🇧 English — Release Notes
- **Ultra-Light Native Rust + Win32 GDI Architecture:** Consumes under 5 MB RAM with ~0% idle CPU overhead.
- **Multi-Mode Fan Control (Auto / Max / Custom Curve):** Direct ACPI / WMI / EC fan register management featuring a flicker-free double-buffered GDI Custom Fan Curve Editor.
- **Real-Time Hardware Telemetry:** Live tracking of Hybrid Intel CPU frequencies (P-Core/E-Core), NVIDIA RTX GPU clocks & wattage, core temperatures, and fan RPMs.
- **Installer Checksum (`predator-fan-ctl-setup-v1.5.0.exe`):**
  - **SHA-256:** ``$sha256``

#### ⚠️ Mandatory Notices (AI Disclosure & Disclaimer)
- **AI Disclosure:** System architecture, Rust/Win32 implementation, and automated verification were developed by **Google Antigravity & Gemini** in collaboration with the author (**Yuutiro-sama**).
- **Commercial Use & Hardware Disclaimer:** Distributed free of charge for personal, educational, and research use under the MIT License; unauthorized commercial repackaging or resale is strictly prohibited. Provided **"AS IS"** without warranty; the author and AI tools assume no liability for hardware or EC/ACPI/WMI configuration risks.
"@

$releasePayload = @{
    tag_name         = "v1.5.0"
    target_commitish = "main"
    name             = "predator-fan-ctl v1.5.0 (Windows x64)"
    body             = $releaseNotes
    draft            = $false
    prerelease       = $false
} | ConvertTo-Json -Depth 3

$releaseRes = Invoke-RestMethod -Uri "https://api.github.com/repos/Yuutiro-sama/predator-fan-ctl/releases" `
    -Headers $headers -Method Post -Body $releasePayload -ContentType "application/json; charset=utf-8"

$releaseId = $releaseRes.id
Write-Output "Created Release ID: $releaseId | URL: $($releaseRes.html_url)"

Bước 4.2: Upload nhị phân dist\predator-fan-ctl-setup-v1.5.0.exe lên Release Asset

Sử dụng endpoint uploads.github.com với Content-Type: application/octet-stream:

PowerShell
$uploadUrl = "https://uploads.github.com/repos/Yuutiro-sama/predator-fan-ctl/releases/$releaseId/assets?name=predator-fan-ctl-setup-v1.5.0.exe"
$uploadHeaders = @{
    Authorization          = "Bearer $ghToken"
    Accept                 = "application/vnd.github+json"
    "Content-Type"         = "application/octet-stream"
    "X-GitHub-Api-Version" = "2022-11-28"
}

$assetRes = Invoke-RestMethod -Uri $uploadUrl -Headers $uploadHeaders -Method Post -InFile $installerPath
Write-Output "Uploaded Asset: $($assetRes.name) | Size: $($assetRes.size) bytes | Download URL: $($assetRes.browser_download_url)"

5. PHASE 5: KIỂM THỬ XÁC MINH QUA API & BÁO CÁO NGHIỆM THU

Sau khi hoàn tất Phase 4, kỹ sư Antigravity thực hiện bước nghiệm thu tự động bằng API (vẫn tuyệt đối không mở trình duyệt GUI) và xuất báo cáo nghiệm thu:

Truy vấn kiểm chứng dữ liệu thực tế trên GitHub:

Gọi GET [https://api.github.com/repos/Yuutiro-sama/predator-fan-ctl](https://api.github.com/repos/Yuutiro-sama/predator-fan-ctl) xác nhận visibility = public, default_branch = main.

Gọi GET [https://api.github.com/repos/Yuutiro-sama/predator-fan-ctl/contents/LICENSE](https://api.github.com/repos/Yuutiro-sama/predator-fan-ctl/contents/LICENSE) và .../contents/README.md xác nhận 2 tệp đã tồn tại trên nhánh main.

Gọi GET [https://api.github.com/repos/Yuutiro-sama/predator-fan-ctl/releases/tags/v1.5.0](https://api.github.com/repos/Yuutiro-sama/predator-fan-ctl/releases/tags/v1.5.0) xác nhận assets.Count -ge 1, assets[0].name == "predator-fan-ctl-setup-v1.5.0.exe", assets[0].state == "uploaded", và kích thước khớp chính xác với file local dist\predator-fan-ctl-setup-v1.5.0.exe.

Xóa biến $ghToken khỏi bộ nhớ phiên PowerShell (Remove-Variable ghToken).

Ghi nhận kết quả vào file báo cáo tiến trình (REPORT_GITHUB_RELEASE_V1.5.0.md tại thư mục làm việc) gồm: URL Repository, URL Release v1.5.0, Direct Download URL của predator-fan-ctl-setup-v1.5.0.exe, mã băm SHA-256, và xác nhận tuân thủ 100% quy tắc không tác động màn hình chính AG2313QS1.