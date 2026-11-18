# Windows smoke test for the edge window, run by CI on windows-latest (a real desktop session).
# Starts the app, then checks with the Win32 API what a user would see and feel:
#   1. it keeps running;
#   2. its window is flush with the right edge of the work area and as tall as it;
#   3. collapsed, only the bar is hit-testable: the bar's centre belongs to the app, a point
#      20 px to its left belongs to whatever is underneath (click-through);
#   4. hovering the bar opens the panel (the window region goes away), leaving closes it.
# Saves screenshots next to the script's working directory for the CI artifact.
param([string]$Exe = "target\debug\edge.exe")
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Collections.Generic;
public static class W {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern int GetWindowRgnBox(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public MOUSEINPUT mi; }
  // Real mouse input, as a hand on the mouse would make it. SetCursorPos only warps the cursor,
  // which WebView2 does not see as movement.
  public static void MoveTo(int x, int y) {
    var i = new INPUT { type = 0 };
    i.mi.dx = (int)((x * 65535L) / (GetSystemMetrics(0) - 1));
    i.mi.dy = (int)((y * 65535L) / (GetSystemMetrics(1) - 1));
    i.mi.flags = 0x0001 | 0x8000; // MOVE | ABSOLUTE
    SendInput(1, new[] { i }, Marshal.SizeOf(typeof(INPUT)));
  }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; public POINT(int x, int y) { X = x; Y = y; } }
  public static IntPtr Find(uint pid) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) { found = h; return false; } return true; }, IntPtr.Zero);
    return found;
  }
  public static IntPtr TopAt(int x, int y) { return GetAncestor(WindowFromPoint(new POINT(x, y)), 2); }
}
"@
[W]::SetProcessDPIAware() | Out-Null  # physical pixels, like the app

$failures = @()
function Check($ok, $what) {
  if ($ok) { Write-Host "ok   $what" } else { Write-Host "FAIL $what"; $script:failures += $what }
}
function Shot($name) {
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  [System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
  $bmp.Save("$PWD\smoke-$name.png")
}

# A fresh CI machine has no notes, so the strip is the bare 120 px bar.
$p = Start-Process -FilePath $Exe -PassThru -RedirectStandardError "$PWD\smoke-app.log"
Start-Sleep -Seconds 20
Check (-not $p.HasExited) "the app is still running after 20 s"
if ($p.HasExited) { exit 1 }

$h = [W]::Find($p.Id)
Check ($h -ne [IntPtr]::Zero) "it has a visible window"
if ($h -eq [IntPtr]::Zero) { Shot "no-window"; exit 1 }

$work = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
$r = New-Object W+RECT
[W]::GetWindowRect($h, [ref]$r) | Out-Null
Write-Host "work area $($work.Left),$($work.Top) $($work.Width)x$($work.Height); window $($r.L),$($r.T)-$($r.R),$($r.B)"
Check ($r.R -eq $work.Right) "the window is flush with the right edge of the work area"
Check (($r.T -eq $work.Top) -and ($r.B -eq $work.Bottom)) "the window spans the work area's height"

$box = New-Object W+RECT
$kind = [W]::GetWindowRgnBox($h, [ref]$box)   # 0 = ERROR (no region), 2 = simple, 3 = complex
Check ($kind -ne 0) "collapsed, the window has a region (only the bar is hit-testable)"
$barW = $box.R - $box.L; $barH = $box.B - $box.T
Write-Host "region $($box.L),$($box.T) ${barW}x${barH} (kind $kind)"
Check (($barW -ge 8) -and ($barW -le 24)) "the region is bar-wide (8 px at 100-300 % scale)"
Check (($barH -ge 120) -and ($barH -le 360)) "the region is bar-tall (120 px at 100-300 % scale)"
Shot "collapsed"

$barX = $r.L + $box.L + [int]($barW / 2); $barY = $r.T + $box.T + [int]($barH / 2)
Check ([W]::TopAt($barX, $barY) -eq $h) "the bar's centre belongs to the app"
Check ([W]::TopAt($barX - 20, $barY) -ne $h) "20 px left of the bar clicks go through to what is underneath"
Check ([W]::TopAt($r.L + 5, $r.T + 5) -ne $h) "the rest of the (transparent) panel area is click-through"

# Hover: the panel opens on pointer enter (hover delay 0) and closes 150 ms after leaving.
[W]::MoveTo($barX - 60, $barY)
Start-Sleep -Milliseconds 300
foreach ($dx in 40, 20, 0) { [W]::MoveTo($barX - $dx, $barY); Start-Sleep -Milliseconds 80 }  # glide onto the bar
$cur = New-Object W+POINT
[W]::GetCursorPos([ref]$cur) | Out-Null
Write-Host "cursor at $($cur.X),$($cur.Y), bar centre $barX,$barY, window under it $([W]::TopAt($cur.X, $cur.Y)) (ours $h)"
Start-Sleep -Seconds 2
$open = [W]::GetWindowRgnBox($h, [ref]$box) -eq 0
Shot "hover"
Check $open "hovering the bar opens the panel (the region is removed)"
Check ([W]::TopAt($r.L + 40, $r.T + 200) -eq $h) "open, the panel takes the pointer"

foreach ($f in 0.8, 0.6, 0.33) { [W]::MoveTo([int]($work.Left + $work.Width * $f), [int]($work.Top + $work.Height / 2)); Start-Sleep -Milliseconds 80 }
Start-Sleep -Seconds 2
Check ([W]::GetWindowRgnBox($h, [ref]$box) -ne 0) "moving away closes it again"
Shot "closed-again"

Stop-Process -Id $p.Id
Start-Sleep -Seconds 1
Write-Host "---- app log (debug build) ----"
Get-Content "$PWD\smoke-app.log" -ErrorAction SilentlyContinue | Select-Object -Last 60
if ($failures.Count) { Write-Host "$($failures.Count) check(s) failed"; exit 1 }
Write-Host "all checks passed"
