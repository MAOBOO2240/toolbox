# A stand-in for the game: a window whose title contains "MapleStory" (that is how the app recognises the game).
# Whatever gets typed into it is written to a file, so the scenario can tell where the keyboard went.
param([string]$Out, [int]$X = 900, [int]$Y = 500)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
# hide this script's own console window, so it does not cover anything
Add-Type -Namespace U -Name N -MemberDefinition '[DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow(); [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);'
[void][U.N]::ShowWindow([U.N]::GetConsoleWindow(), 0)
$f = New-Object System.Windows.Forms.Form
$f.Text = 'MapleStory'
$f.StartPosition = 'Manual'
$f.Location = New-Object System.Drawing.Point($X, $Y)
$f.Size = New-Object System.Drawing.Size(460, 240)
$t = New-Object System.Windows.Forms.TextBox
$t.Multiline = $true
$t.Dock = 'Fill'
$t.Font = New-Object System.Drawing.Font('Consolas', 16)
$t.Add_TextChanged({ [System.IO.File]::WriteAllText($Out, $t.Text) })
$f.Controls.Add($t)
$f.Add_Shown({ $f.Activate(); $t.Focus() })
[System.IO.File]::WriteAllText($Out, '')
[System.Windows.Forms.Application]::Run($f)
