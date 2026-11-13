# UI check on a Windows runner: start the test build, click through the browser window, save screenshots.
# ASCII only (Windows PowerShell reads a script without BOM as ANSI).
$ErrorActionPreference = 'Continue'
$root = $env:GITHUB_WORKSPACE
if (-not $root) { $root = (Get-Location).Path }
$shots = Join-Path $root 'shots'
New-Item -ItemType Directory -Force -Path $shots | Out-Null
$log = Join-Path $shots 'log.txt'
function Log([string]$m) {
  $line = ('[{0:HH:mm:ss.fff}] {1}' -f (Get-Date), $m)
  Add-Content -Path $log -Value $line -Encoding UTF8
  Write-Host $line
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern short VkKeyScan(char ch);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hgt, bool repaint);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  public class Win { public IntPtr H; public bool Visible; public int L, T, R, B; public string Class; public string Title; public int Ex; public int Style; public uint Pid; }
  public static Win Info(IntPtr h) {
    uint p; GetWindowThreadProcessId(h, out p);
    RECT r; GetWindowRect(h, out r);
    var t = new StringBuilder(256); GetWindowText(h, t, 256);
    var c = new StringBuilder(256); GetClassName(h, c, 256);
    var w = new Win(); w.H = h; w.Visible = IsWindowVisible(h); w.L = r.L; w.T = r.T; w.R = r.R; w.B = r.B;
    w.Class = c.ToString(); w.Title = t.ToString(); w.Ex = GetWindowLong(h, -20); w.Style = GetWindowLong(h, -16); w.Pid = p;
    return w;
  }
  public static List<Win> List(uint[] pids) {
    var o = new List<Win>();
    EnumWindows(delegate(IntPtr h, IntPtr l) {
      uint p; GetWindowThreadProcessId(h, out p);
      if (Array.IndexOf(pids, p) >= 0) o.Add(Info(h));
      return true;
    }, IntPtr.Zero);
    return o;
  }
  public static void TypeText(string s, int delay) {
    foreach (char ch in s) {
      short v = VkKeyScan(ch);
      if (v == -1) continue;
      byte vk = (byte)(v & 0xff);
      bool shift = (v & 0x100) != 0;
      if (shift) keybd_event(0x10, 0, 0, UIntPtr.Zero);
      keybd_event(vk, 0, 0, UIntPtr.Zero);
      keybd_event(vk, 0, 2, UIntPtr.Zero);
      if (shift) keybd_event(0x10, 0, 2, UIntPtr.Zero);
      System.Threading.Thread.Sleep(delay);
    }
  }
}
'@
[void][W]::SetProcessDPIAware()

# ---------- helpers ----------
function Pids { @(Get-Process -Name 'MAOBOO' -ErrorAction SilentlyContinue | ForEach-Object { [uint32]$_.Id }) }
function Wins {
  $p = Pids
  if ($p.Count -eq 0) { return @() }
  return @([W]::List([uint32[]]$p))
}
function Describe($w) {
  return ('h={0} vis={1} [{2},{3},{4},{5}] {6}x{7} ex={8:X8} st={9:X8} title="{10}"' -f $w.H, [int]$w.Visible, $w.L, $w.T, $w.R, $w.B, ($w.R - $w.L), ($w.B - $w.T), $w.Ex, $w.Style, $w.Title)
}
function Fg {
  $h = [W]::GetForegroundWindow()
  if ($h -eq [IntPtr]::Zero) { return 'fg=(none)' }
  $i = [W]::Info($h)
  $name = ''
  try { $name = (Get-Process -Id $i.Pid -ErrorAction Stop).ProcessName } catch {}
  return ('fg h={0} proc={1} title="{2}"' -f $h, $name, $i.Title)
}
function LogWins([string]$tag) {
  Log ('-- ' + $tag + ' | ' + (Fg))
  foreach ($w in (Wins)) {
    if ($w.Visible) { Log ('   ' + (Describe $w)) }
  }
}
function Shot([string]$name) {
  try {
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save((Join-Path $shots ($name + '.png')), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
  } catch { Log ('shot failed: ' + $_) }
  $p = [System.Windows.Forms.Cursor]::Position
  LogWins ($name + (' cursor=({0},{1})' -f $p.X, $p.Y))
}
function Move([int]$x, [int]$y) {
  $p = [System.Windows.Forms.Cursor]::Position
  for ($i = 1; $i -le 4; $i++) {
    [void][W]::SetCursorPos([int]($p.X + ($x - $p.X) * $i / 4), [int]($p.Y + ($y - $p.Y) * $i / 4))
    [W]::mouse_event(0x0001, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 25
  }
  [void][W]::SetCursorPos($x, $y)
  [W]::mouse_event(0x0001, 0, 0, 0, [UIntPtr]::Zero)
}
function Click([int]$x, [int]$y, [string]$button = 'left') {
  Move $x $y
  Start-Sleep -Milliseconds 150
  if ($button -eq 'right') { $d = 0x0008; $u = 0x0010 } elseif ($button -eq 'middle') { $d = 0x0020; $u = 0x0040 } else { $d = 0x0002; $u = 0x0004 }
  [W]::mouse_event($d, 0, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 50
  [W]::mouse_event($u, 0, 0, 0, [UIntPtr]::Zero)
}
function Key([int]$vk) {
  [W]::keybd_event([byte]$vk, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 40
  [W]::keybd_event([byte]$vk, 0, 2, [UIntPtr]::Zero)
}
function Chord([int[]]$mods, [int]$vk) {
  foreach ($m in $mods) { [W]::keybd_event([byte]$m, 0, 0, [UIntPtr]::Zero) }
  Start-Sleep -Milliseconds 40
  Key $vk
  Start-Sleep -Milliseconds 40
  foreach ($m in $mods) { [W]::keybd_event([byte]$m, 0, 2, [UIntPtr]::Zero) }
}
function TypeText([string]$s) { [W]::TypeText($s, 35) }
function Wait([int]$ms) { Start-Sleep -Milliseconds $ms }

$VK_ENTER = 0x0D; $VK_ESC = 0x1B; $VK_CTRL = 0x11; $VK_SHIFT = 0x10; $VK_F11 = 0x7A

# ---------- environment ----------
try { Set-DisplayResolution -Width 1920 -Height 1080 -Force | Out-Null } catch { Log ('Set-DisplayResolution: ' + $_) }
Wait 1500
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Log ('screen ' + $screen.Width + 'x' + $screen.Height)
$big = ($screen.Width -ge 1600)
if ($big) { $BX = 10; $BY = 10; $BW = 1100; $BH = 760 } else { $BX = 8; $BY = 8; $BW = 1000; $BH = 720 }

# keep the usage report away from the author's dashboard: this run must not show up as a computer there
try {
  $src = Get-Content (Join-Path $root 'src-tauri/src/stats.rs') -Raw
  if ($src -match 'STATS_URL: &str = "https://([^"/]+)') {
    Add-Content -Path "$env:SystemRoot\System32\drivers\etc\hosts" -Value ("`r`n0.0.0.0 " + $Matches[1])
    ipconfig /flushdns | Out-Null
    Log 'stats host blocked'
  }
} catch { Log ('hosts: ' + $_) }

# browser state: one window, open at a known place
$data = Join-Path $env:APPDATA 'com.toolbox.desktop'
New-Item -ItemType Directory -Force -Path $data | Out-Null
$web = '{"next_id":5,"wins":[{"id":"w1","x":' + $BX + ',"y":' + $BY + ',"w":' + $BW + ',"h":' + $BH + ',"open":true}]}'
[System.IO.File]::WriteAllText((Join-Path $data 'web.json'), $web)

# a Chrome profile with a few bookmarks, so the automatic sync has something to bring in
$chrome = Join-Path $env:LOCALAPPDATA 'Google\Chrome\User Data'
if (-not (Test-Path (Join-Path $chrome 'Default\Bookmarks'))) {
  New-Item -ItemType Directory -Force -Path (Join-Path $chrome 'Default') | Out-Null
  $bm = @'
{"roots":{"bookmark_bar":{"children":[
{"type":"url","id":"5","name":"YouTube","url":"https://www.youtube.com/"},
{"type":"url","id":"6","name":"Example","url":"https://example.com/"},
{"type":"folder","id":"7","name":"\u6953\u4e4b\u8c37","children":[
  {"type":"url","id":"8","name":"\u5b98\u7db2","url":"https://maplestory.beanfun.com/"},
  {"type":"folder","id":"9","name":"\u653b\u7565","children":[{"type":"url","id":"10","name":"Wikipedia","url":"https://www.wikipedia.org/"}]},
  {"type":"url","id":"11","name":"IANA","url":"https://www.iana.org/"}]},
{"type":"url","id":"12","name":"GitHub","url":"https://github.com/"},
{"type":"url","id":"13","name":"Bing","url":"https://www.bing.com/"}
],"id":"1","name":"Bookmarks bar","type":"folder"},
"other":{"children":[{"type":"url","id":"14","name":"MDN","url":"https://developer.mozilla.org/"}],"id":"2","name":"Other bookmarks","type":"folder"},
"synced":{"children":[],"id":"3","name":"Mobile bookmarks","type":"folder"}},"version":1}
'@
  [System.IO.File]::WriteAllText((Join-Path $chrome 'Default\Bookmarks'), $bm)
  if (-not (Test-Path (Join-Path $chrome 'Local State'))) {
    [System.IO.File]::WriteAllText((Join-Path $chrome 'Local State'), '{"profile":{"info_cache":{"Default":{"name":"Tester"}},"last_used":"Default"}}')
  }
  Log 'chrome profile seeded'
} else { Log 'chrome profile already there' }

# ---------- start ----------
$exe = Get-ChildItem (Join-Path $root 'app') -Filter 'MAOBOO.exe' -Recurse | Select-Object -First 1
if (-not $exe) { Log 'MAOBOO.exe not found'; exit 0 }
Log ('starting ' + $exe.FullName)
Start-Process -FilePath $exe.FullName -WorkingDirectory $exe.DirectoryName
$browser = $null
for ($i = 0; $i -lt 16 -and -not $browser; $i++) {
  Wait 2500
  foreach ($w in (Wins)) {
    if ($w.Visible -and [Math]::Abs(($w.R - $w.L) - $BW) -le 24 -and [Math]::Abs(($w.B - $w.T) - $BH) -le 24) { $browser = $w.H }
  }
  LogWins ('boot ' + $i)
}
Wait 6000
Shot '01-start'
if (-not $browser) { Log 'browser window not found'; Shot '01b-nobrowser'; Get-Process | Where-Object { $_.ProcessName -match 'MAOBOO|msedgewebview2' } | ForEach-Object { Log ('proc ' + $_.ProcessName + ' ' + $_.Id) }; exit 0 }

function R { return [W]::Info($browser) }
# a point in the browser window: negative x counts from the right edge, negative y from the bottom edge
function P([int]$dx, [int]$dy) {
  $r = R
  if ($dx -ge 0) { $x = $r.L + $dx } else { $x = $r.R + $dx }
  if ($dy -ge 0) { $y = $r.T + $dy } else { $y = $r.B + $dy }
  return @($x, $y)
}
function CB([int]$dx, [int]$dy, [string]$button = 'left') { $p = P $dx $dy; Click $p[0] $p[1] $button }
function MB([int]$dx, [int]$dy) { $p = P $dx $dy; Move $p[0] $p[1] }
function Popups { @(Wins | Where-Object { $_.Visible -and $_.H -ne $browser -and (($_.Ex -band 0x08000000) -ne 0) }) }
function Dismiss { CB -70 -70; Wait 500 }

# the main program window: move it out of the way (it stays the foreground window, like when you click the program)
$main = $null
foreach ($w in (Wins)) { if ($w.Visible -and $w.H -ne $browser -and (($w.Ex -band 0x08000000) -eq 0) -and $w.Title -and ($w.R - $w.L) -gt 300) { $main = $w } }
if ($main) {
  if ($big) { [void][W]::MoveWindow($main.H, 1130, 20, ($main.R - $main.L), ($main.B - $main.T), $true) }
  else { [void][W]::MoveWindow($main.H, 700, 560, ($main.R - $main.L), ($main.B - $main.T), $true) }
  Wait 800
}
Shot '02-ready'

# ---------- part 1: the program itself is the foreground window ----------
# main menu
CB -23 71; Wait 900
Shot '03-menu'
$pp = @(Popups)
if ($pp.Count -gt 0) {
  $m = $pp[0]
  $seen = 0
  for ($y = $m.T + 18; $y -lt $m.B - 8 -and $seen -lt 7; $y += 22) {
    Move ([int](($m.L + $m.R) / 2)) $y
    Wait 420
    $now = @(Popups).Count
    if ($now -gt 1) {
      $seen++
      Shot ('04-sub-' + $seen + '-y' + ($y - $m.T))
      # into the submenu and back, to see that it stays open while the mouse travels
      $sub = (Popups | Where-Object { $_.H -ne $m.H } | Select-Object -First 1)
      if ($sub -and $seen -eq 1) { Move ([int](($sub.L + $sub.R) / 2)) ($sub.T + 24); Wait 500; Shot '04b-in-sub' }
      # skip past this row
      $y += 12
    }
  }
} else { Log 'no popup after clicking the menu button' }
Dismiss
LogWins 'after menu dismissed'

# address bar: typing, suggestions, going to a page
CB 520 71; Wait 700
Shot '05-omni-focus'
TypeText 'exa'; Wait 1800
Shot '06-omni-typing'
TypeText 'mple.com'; Wait 400
Key $VK_ENTER; Wait 7000
Shot '07-page'

# page context menu
CB 420 420 'right'; Wait 900
Shot '08-page-context'
Dismiss
# link context menu (example.com has one link near the bottom of its text)
CB 120 330 'right'; Wait 900
Shot '08b-context-2'
Dismiss

# tab context menu
CB 150 31 'right'; Wait 900
Shot '09-tab-menu'
Dismiss

# new tab button
CB 292 31; Wait 2500
Shot '10-newtab'

# internal pages
$n = 11
foreach ($page in @('history', 'downloads', 'bookmarks', 'extensions', 'settings')) {
  CB 520 71; Wait 500
  TypeText ('chrome://' + $page); Wait 300
  Key $VK_ENTER; Wait 3000
  Shot ('' + $n + '-' + $page)
  $n++
}
# settings: scroll down a little to see more
MB 500 500; [W]::mouse_event(0x0800, 0, 0, [uint32]4294966576, [UIntPtr]::Zero); Wait 900
Shot '15b-settings-scrolled'

# toolbar buttons
CB -59 71; Wait 900; Shot '16-profile'; Dismiss
CB -95 71; Wait 900; Shot '17-extensions-menu'; Dismiss
# back to the first tab (example.com)
CB 150 31; Wait 1500
CB -217 71; Wait 900; Shot '18-bookmark-bubble'; Dismiss
CB 174 71; Wait 900; Shot '19-site-info'; Dismiss
CB 20 31; Wait 900; Shot '20-tab-search'; Dismiss

# bookmarks bar: folder, nested folder, right click
CB 190 111; Wait 900; Shot '21-bm-folder'
$pp = @(Popups)
if ($pp.Count -gt 0) { $m = $pp[0]; Move ([int](($m.L + $m.R) / 2)) ($m.T + 52); Wait 700; Shot '21b-bm-nested' }
Dismiss
CB 40 111 'right'; Wait 900; Shot '22-bm-context'; Dismiss

# maximize by double click on the empty part of the tab strip, then restore with the button
$p = P -330 10
Click $p[0] $p[1]; Wait 120; Click $p[0] $p[1]; Wait 1800
Shot '23-maximized'
Log ('zoomed=' + [W]::IsZoomed($browser) + ' ' + (Describe (R)))
CB -69 16; Wait 1800
Shot '24-restored'
Log ('zoomed=' + [W]::IsZoomed($browser) + ' ' + (Describe (R)))

# keyboard shortcuts while the page has the keyboard
CB 500 480; Wait 600
Chord @($VK_CTRL) 0x54; Wait 1800      # Ctrl+T
Shot '25-ctrl-t'
Chord @($VK_CTRL) 0x57; Wait 1200      # Ctrl+W
CB 500 480; Wait 500
Chord @($VK_CTRL) 0x48; Wait 2500      # Ctrl+H
Shot '26-ctrl-h'
CB 500 480; Wait 500
Chord @($VK_CTRL) 0x46; Wait 1200      # Ctrl+F
Shot '27-ctrl-f'
Key $VK_ESC; Wait 500
Key $VK_F11; Wait 2200
Shot '28-f11'
Log ('f11 ' + (Describe (R)))
Key $VK_F11; Wait 2200
Shot '29-f11-off'
Log ('f11 off ' + (Describe (R)))
Chord @($VK_CTRL, $VK_SHIFT) 0x4E; Wait 5000   # Ctrl+Shift+N
Shot '30-incognito'
# close the incognito window with its own close button (it is the other big window that is not the main program)
$inc = $null
foreach ($w in (Wins)) { if ($w.Visible -and $w.H -ne $browser -and (-not $main -or $w.H -ne $main.H) -and (($w.Ex -band 0x08000000) -eq 0) -and ($w.R - $w.L) -gt 300) { $inc = $w } }
if ($inc) { Log ('incognito ' + (Describe $inc)); Click ($inc.R - 23) ($inc.T + 16); Wait 1500 } else { Log 'no incognito window found' }
Shot '31-after-incognito'

# ---------- part 2: a "game" has the keyboard ----------
$gameOut = Join-Path $shots 'game-input.txt'
if ($big) { $gx = 1130; $gy = 640 } else { $gx = 540; $gy = 500 }
Start-Process -FilePath 'powershell.exe' -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + (Join-Path $PSScriptRoot 'game.ps1') + '"'), '-Out', ('"' + $gameOut + '"'), '-X', $gx, '-Y', $gy)
Wait 5000
function GameText { try { return [System.IO.File]::ReadAllText($gameOut) } catch { return '(unreadable)' } }
# click into the game: it becomes the foreground window and has the keyboard
Click ($gx + 200) ($gy + 120); Wait 800
TypeText 'abc'; Wait 500
Log ('G1 game focused, typed abc | ' + (Fg) + ' | game text="' + (GameText) + '"')
Shot '40-game'
# click on the page: the game must keep the keyboard
CB 420 520; Wait 800
TypeText 'def'; Wait 500
Log ('G2 clicked the page, typed def | ' + (Fg) + ' | game text="' + (GameText) + '"')
# open the main menu and pick "new tab" with the mouse: still the game
CB -23 71; Wait 900
Shot '41-game-menu'
$pp = @(Popups)
if ($pp.Count -gt 0) { Click ($pp[0].L + 80) ($pp[0].T + 22); Wait 2000 } else { Log 'G3 no menu popup' }
TypeText 'ghi'; Wait 500
Log ('G3 picked new tab from the menu, typed ghi | ' + (Fg) + ' | game text="' + (GameText) + '"')
Shot '42-game-newtab'
# click the address bar: now the browser takes the keyboard
CB 520 71; Wait 900
Log ('G4 clicked the address bar | ' + (Fg))
TypeText 'iana.org'; Wait 1200
Shot '43-game-typing'
Log ('G5 typed iana.org | ' + (Fg) + ' | game text="' + (GameText) + '"')
Key $VK_ENTER; Wait 6000
Log ('G6 pressed Enter | ' + (Fg) + ' | browser title="' + (R).Title + '"')
TypeText 'jkl'; Wait 500
Log ('G7 typed jkl | ' + (Fg) + ' | game text="' + (GameText) + '"')
Shot '44-game-after-enter'
# scroll the page with the wheel, right click it, pick nothing
MB 420 520; [W]::mouse_event(0x0800, 0, 0, [uint32]4294966936, [UIntPtr]::Zero); Wait 700
CB 420 520 'right'; Wait 900
Shot '45-game-context'
Log ('G8 right clicked the page | ' + (Fg))
Dismiss
TypeText 'mno'; Wait 500
Log ('G9 typed mno | ' + (Fg) + ' | game text="' + (GameText) + '"')

# ---------- the end ----------
CB -23 16; Wait 2000
Shot '50-closed'
foreach ($f in @('web.json', 'web-history.json', 'web-downloads.json')) {
  $src = Join-Path $data $f
  if (Test-Path $src) { Copy-Item $src (Join-Path $shots ('data-' + $f)) }
}
Get-Process -Name 'MAOBOO' -ErrorAction SilentlyContinue | Stop-Process -Force
Get-Process -Name 'powershell' -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $PID -and $_.MainWindowTitle -eq 'MapleStory' } | Stop-Process -Force
Log 'done'
exit 0
