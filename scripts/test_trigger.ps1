$signature = @"
using System;
using System.Runtime.InteropServices;
public class WinTest {
    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
    public static extern IntPtr FindWindow(string lpClassName, string lpWindowName);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);
}
"@
Add-Type -TypeDefinition $signature -ErrorAction SilentlyContinue

$hwnd = [WinTest]::FindWindow("PredatorFanTrayWindowClass", $null)
$out = "Tray HWND: $hwnd"
if ($hwnd -ne [IntPtr]::Zero) {
    $ok = [WinTest]::PostMessage($hwnd, 0x0111, [IntPtr]1050, [IntPtr]::Zero)
    $out += " | PostMessage IDM_SHOW_TEMP_CHART (1050): $ok"
}
[System.IO.File]::WriteAllText("$PSScriptRoot\trigger_result.txt", $out)
