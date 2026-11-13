$signature = @"
using System;
using System.Runtime.InteropServices;
public class WinCheck {
    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
    public static extern IntPtr FindWindow(string lpClassName, string lpWindowName);
}
"@
Add-Type -TypeDefinition $signature -ErrorAction SilentlyContinue
$hwnd = [WinCheck]::FindWindow("PredatorFanTrayWindowClass", $null)
$err = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
Write-Host "HWND: $hwnd, Error: $err"
