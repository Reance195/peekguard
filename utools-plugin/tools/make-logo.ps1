# 生成 uTools 插件使用的 logo.png（256x256，透明背景）。
# 设计跟随 src/icon_raster.rs 的眼睛图标：深色椭圆环 + 青色瞳孔 + 白色高光
# + 青色上眼皮弧线。
#
# 用法（推荐 pwsh，避免 Windows PowerShell 的编码问题）：
#   pwsh -File make-logo.ps1 [-Out 输出路径]

param(
    [string]$Out = (Join-Path (Split-Path -Parent $PSScriptRoot) 'logo.png')
)

Add-Type -AssemblyName System.Drawing

$size = 256
$bmp = New-Object System.Drawing.Bitmap($size, $size)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.Clear([System.Drawing.Color]::Transparent)

$rim = [System.Drawing.Color]::FromArgb(255, 26, 28, 36)      # 深色轮廓 0.10,0.11,0.14
$cyan = [System.Drawing.Color]::FromArgb(255, 41, 184, 255)   # 青色瞳孔 0.16,0.72,1.00
$white = [System.Drawing.Color]::White

# 眼睛轮廓：深色椭圆环
$eye = New-Object System.Drawing.RectangleF(30, 78, 196, 100)
$penRim = New-Object System.Drawing.Pen($rim, 14)
$g.DrawEllipse($penRim, $eye)

# 青色瞳孔（居中）
$r = 27
$brushCyan = New-Object System.Drawing.SolidBrush($cyan)
$g.FillEllipse($brushCyan, (128 - $r), (128 - $r), ($r * 2), ($r * 2))

# 白色高光（偏左上）
$brushWhite = New-Object System.Drawing.SolidBrush($white)
$g.FillEllipse($brushWhite, (128 - 16), (128 - 22), 17, 17)

# 青色上眼皮弧线（内缩一圈，覆盖上半部分）
$inner = New-Object System.Drawing.RectangleF(37, 85, 182, 86)
$penLid = New-Object System.Drawing.Pen($cyan, 4)
$g.DrawArc($penLid, $inner, 195, 150)

$g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output "logo written: $Out"
