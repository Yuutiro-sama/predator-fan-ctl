# scripts/manage_restore_points.ps1
# Tự động tạo điểm khôi phục dễ phân biệt và dọn dẹp các điểm cũ, chỉ giữ lại đúng 3 bản gần nhất.

$logFile = "C:\Users\default.LAPTOP-1V8CRH32\Documents\antigravity\fan-control\restore_point_log.txt"
"=== BẮT ĐẦU QUẢN LÝ ĐIỂM KHÔI PHỤC (SYSTEM RESTORE & SHADOW COPIES) ===" | Out-File -FilePath $logFile -Encoding utf8

try {
    # 1. Bỏ giới hạn tần suất tạo điểm khôi phục (mặc định Windows giới hạn 1 lần / 24h)
    $regPath = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore"
    if (Test-Path $regPath) {
        Set-ItemProperty -Path $regPath -Name "SystemRestorePointCreationFrequency" -Value 0 -Type DWord -Force -ErrorAction SilentlyContinue
    }

    # 2. Tạo điểm khôi phục mới với tên rõ ràng, dễ phân biệt
    $rpName = "PREDATOR_THERMAL_GOVERNOR_BACKUP"
    "[*] Đang tạo điểm khôi phục mới: '$rpName'..." | Out-File -FilePath $logFile -Append -Encoding utf8
    $newRp = Checkpoint-Computer -Description $rpName -RestorePointType "MODIFY_SETTINGS" -ErrorAction Stop
    "[+] Điểm khôi phục mới đã tạo thành công!" | Out-File -FilePath $logFile -Append -Encoding utf8

} catch {
    "[!] Lỗi khi gọi Checkpoint-Computer: $($_.Exception.Message)" | Out-File -FilePath $logFile -Append -Encoding utf8
}

# 3. Quét danh sách Volume Shadow Copies tương ứng với các Restore Points
$shadows = Get-CimInstance Win32_ShadowCopy | Sort-Object InstallDate -Descending
"[*] Tổng số Shadow Copies hiện có: $($shadows.Count)" | Out-File -FilePath $logFile -Append -Encoding utf8

# Giữ lại 3 bản mới nhất, xóa các bản cũ hơn
if ($shadows.Count -gt 3) {
    $toKeep = $shadows | Select-Object -First 3
    $toDelete = $shadows | Select-Object -Skip 3

    "[*] Các bản được GIỮ LẠI (3 bản mới nhất):" | Out-File -FilePath $logFile -Append -Encoding utf8
    foreach ($k in $toKeep) {
        "   - ID: $($k.ID) | Ngày: $($k.InstallDate) | Device: $($k.DeviceObject)" | Out-File -FilePath $logFile -Append -Encoding utf8
    }

    "[*] Đang xóa các bản cũ hơn để chừa lại đúng 3 bản gần nhất:" | Out-File -FilePath $logFile -Append -Encoding utf8
    foreach ($d in $toDelete) {
        try {
            "   - Xóa ID: $($d.ID) (Ngày: $($d.InstallDate))..." | Out-File -FilePath $logFile -Append -Encoding utf8
            vssadmin delete shadows /Shadow="$($d.ID)" /Quiet | Out-Null
            "[+] Đã xóa thành công shadow copy: $($d.ID)" | Out-File -FilePath $logFile -Append -Encoding utf8
        } catch {
            "[!] Lỗi xóa shadow copy $($d.ID): $($_.Exception.Message)" | Out-File -FilePath $logFile -Append -Encoding utf8
        }
    }
} else {
    "[*] Hiện tại có $($shadows.Count) bản (<= 3), không cần xóa bản cũ." | Out-File -FilePath $logFile -Append -Encoding utf8
}

# 4. Xuất danh sách Restore Points và Shadow Copies cuối cùng
"`n=== DANH SÁCH RESTORE POINTS HIỆN TẠI TRÊN HỆ THỐNG ===" | Out-File -FilePath $logFile -Append -Encoding utf8
Get-ComputerRestorePoint | Select-Object SequenceNumber, Description, CreationTime, EventType | Format-Table -AutoSize | Out-String | Out-File -FilePath $logFile -Append -Encoding utf8

"`n=== DANH SÁCH SHADOW COPIES HIỆN TẠI ===" | Out-File -FilePath $logFile -Append -Encoding utf8
Get-CimInstance Win32_ShadowCopy | Select-Object ID, InstallDate, DeviceObject | Format-Table -AutoSize | Out-String | Out-File -FilePath $logFile -Append -Encoding utf8

"=== HOÀN TẤT THÀNH CÔNG ===" | Out-File -FilePath $logFile -Append -Encoding utf8
