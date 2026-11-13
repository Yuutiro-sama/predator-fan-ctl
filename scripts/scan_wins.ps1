$signature = @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class WinScan {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll", CharSet = CharSet.Auto)]
    public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);
    [DllImport("user32.dll", CharSet = CharSet.Auto)]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    public static List<string> ScanAll() {
        var list = new List<string>();
        EnumWindows((hWnd, lParam) => {
            uint pid;
            GetWindowThreadProcessId(hWnd, out pid);
            var sbClass = new StringBuilder(256);
            GetClassName(hWnd, sbClass, 256);
            var sbText = new StringBuilder(256);
            GetWindowText(hWnd, sbText, 256);
            string c = sbClass.ToString();
            string t = sbText.ToString();
            if (c.IndexOf("predator", StringComparison.OrdinalIgnoreCase) >= 0 ||
                t.IndexOf("predator", StringComparison.OrdinalIgnoreCase) >= 0 ||
                pid == 21828) {
                list.Add(string.Format("PID: {0}, HWND: 0x{1:X}, Class: '{2}', Title: '{3}'", pid, hWnd.ToInt64(), c, t));
            }
            return true;
        }, IntPtr.Zero);
        return list;
    }
}
"@
Add-Type -TypeDefinition $signature -ErrorAction SilentlyContinue
$res = [WinScan]::ScanAll()
$res | Out-File "$PSScriptRoot\scan_result.txt"
