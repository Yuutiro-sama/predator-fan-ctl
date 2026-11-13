$signature = @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class ThreadWin {
    public delegate bool EnumThreadDelegate(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")]
    public static extern bool EnumThreadWindows(int dwThreadId, EnumThreadDelegate lpfn, IntPtr lParam);
    [DllImport("user32.dll", CharSet = CharSet.Auto)]
    public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);
    [DllImport("user32.dll", CharSet = CharSet.Auto)]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessage(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);

    public static List<string> CheckThread(int tid) {
        var res = new List<string>();
        EnumThreadWindows(tid, (h, l) => {
            var c = new StringBuilder(256);
            GetClassName(h, c, 256);
            var t = new StringBuilder(256);
            GetWindowText(h, t, 256);
            res.Add(string.Format("HWND: 0x{0:X}, Class: '{1}', Title: '{2}'", h.ToInt64(), c.ToString(), t.ToString()));
            if (c.ToString().Contains("PredatorFanTray")) {
                PostMessage(h, 0x0111, (IntPtr)1050, IntPtr.Zero);
            }
            return true;
        }, IntPtr.Zero);
        return res;
    }
}
"@
Add-Type -TypeDefinition $signature -ErrorAction SilentlyContinue
$tids = (Get-Process -Name predator-fan-ctl).Threads.Id
$all = foreach ($tid in $tids) {
    [ThreadWin]::CheckThread($tid)
}
$all | Out-File "$PSScriptRoot\thread_wins.txt"
