$signature = @"
using System;
using System.Runtime.InteropServices;
using System.Text;

public class DesktopCheck {
    [DllImport("user32.dll")]
    public static extern IntPtr GetProcessWindowStation();
    [DllImport("user32.dll")]
    public static extern IntPtr GetThreadDesktop(int dwThreadId);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool GetUserObjectInformation(IntPtr hObj, int nIndex, StringBuilder pvInfo, int nLength, out int lpnLengthNeeded);

    public static string GetInfo() {
        var hWinSta = GetProcessWindowStation();
        var sb = new StringBuilder(256);
        int len;
        GetUserObjectInformation(hWinSta, 2, sb, 256, out len);
        return "WinStation: " + sb.ToString();
    }
}
"@
Add-Type -TypeDefinition $signature -ErrorAction SilentlyContinue
[DesktopCheck]::GetInfo()
