# Generate Windows ICO derivatives from the shared SVG source assets.
#
# The source of truth stays in common/assets. This script only writes
# platform-specific build artifacts under assets/windows.

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$sourceRoot = Join-Path $repoRoot 'common\assets'
$outputRoot = Join-Path $repoRoot 'assets\windows'
$edgeCandidates = @(
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
    "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe"
)
$edge = $edgeCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $edge) {
    throw 'Microsoft Edge is required to rasterize the shared SVG icons.'
}

New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
Add-Type -AssemblyName System.Drawing

function Convert-PngToIco {
    param(
        [Parameter(Mandatory)][string]$PngPath,
        [Parameter(Mandatory)][string]$IcoPath,
        [switch]$TrimTransparent
    )

    $sizes = @(16, 20, 24, 32, 40, 48, 64, 256)
    $source = [System.Drawing.Image]::FromFile($PngPath)
    $sourceRect = [System.Drawing.Rectangle]::new(0, 0, $source.Width, $source.Height)
    if ($TrimTransparent) {
        $scan = [System.Drawing.Bitmap]::new($source)
        try {
            $left = $scan.Width
            $top = $scan.Height
            $right = -1
            $bottom = -1
            for ($y = 0; $y -lt $scan.Height; $y++) {
                for ($x = 0; $x -lt $scan.Width; $x++) {
                    if ($scan.GetPixel($x, $y).A -gt 0) {
                        $left = [Math]::Min($left, $x)
                        $top = [Math]::Min($top, $y)
                        $right = [Math]::Max($right, $x)
                        $bottom = [Math]::Max($bottom, $y)
                    }
                }
            }
            if ($right -ge $left -and $bottom -ge $top) {
                $contentWidth = $right - $left + 1
                $contentHeight = $bottom - $top + 1
                $side = [Math]::Ceiling([Math]::Max($contentWidth, $contentHeight) * 1.08)
                $centerX = ($left + $right) / 2
                $centerY = ($top + $bottom) / 2
                $cropLeft = [Math]::Max(0, [Math]::Floor($centerX - ($side / 2)))
                $cropTop = [Math]::Max(0, [Math]::Floor($centerY - ($side / 2)))
                $side = [Math]::Min($side, [Math]::Min($scan.Width - $cropLeft, $scan.Height - $cropTop))
                $sourceRect = [System.Drawing.Rectangle]::new($cropLeft, $cropTop, $side, $side)
            }
        } finally {
            $scan.Dispose()
        }
    }
    $images = [System.Collections.Generic.List[byte[]]]::new()
    try {
        foreach ($size in $sizes) {
            $bitmap = [System.Drawing.Bitmap]::new(
                $size,
                $size,
                [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
            )
            try {
                $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
                try {
                    $graphics.Clear([System.Drawing.Color]::Transparent)
                    $graphics.CompositingMode =
                        [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
                    $graphics.CompositingQuality =
                        [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
                    $graphics.InterpolationMode =
                        [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                    $graphics.SmoothingMode =
                        [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                    $graphics.PixelOffsetMode =
                        [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                    $destination = [System.Drawing.Rectangle]::new(0, 0, $size, $size)
                    $graphics.DrawImage(
                        $source,
                        $destination,
                        $sourceRect,
                        [System.Drawing.GraphicsUnit]::Pixel
                    )
                } finally {
                    $graphics.Dispose()
                }
                $stream = [System.IO.MemoryStream]::new()
                try {
                    $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
                    $images.Add($stream.ToArray())
                } finally {
                    $stream.Dispose()
                }
            } finally {
                $bitmap.Dispose()
            }
        }
    } finally {
        $source.Dispose()
    }

    $file = [System.IO.File]::Open(
        $IcoPath,
        [System.IO.FileMode]::Create,
        [System.IO.FileAccess]::Write
    )
    $writer = [System.IO.BinaryWriter]::new($file)
    try {
        $writer.Write([uint16]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]$images.Count)
        $offset = 6 + (16 * $images.Count)
        for ($i = 0; $i -lt $images.Count; $i++) {
            $size = $sizes[$i]
            $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
            $writer.Write([byte]$(if ($size -eq 256) { 0 } else { $size }))
            $writer.Write([byte]0)
            $writer.Write([byte]0)
            $writer.Write([uint16]1)
            $writer.Write([uint16]32)
            $writer.Write([uint32]$images[$i].Length)
            $writer.Write([uint32]$offset)
            $offset += $images[$i].Length
        }
        foreach ($image in $images) {
            $writer.Write($image)
        }
    } finally {
        $writer.Dispose()
        $file.Dispose()
    }
}

$icons = [ordered]@{
    "$([char]0x6F88).svg" = 'cheime.ico'
    'zh-black.svg' = 'zh-black.ico'
    'zh-white.svg' = 'zh-white.ico'
    'en-black.svg' = 'en-black.ico'
    'en-white.svg' = 'en-white.ico'
}

$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) "cheime-icons-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null
try {
    foreach ($entry in $icons.GetEnumerator()) {
        $svgPath = (Resolve-Path -LiteralPath (Join-Path $sourceRoot $entry.Key)).Path
        $pngPath = Join-Path $tempRoot "$($entry.Value).png"
        $fileUrl = "file:///$($svgPath.Replace('\', '/'))"
        $process = Start-Process -FilePath $edge -ArgumentList @(
            '--headless=new',
            '--disable-gpu',
            '--hide-scrollbars',
            '--default-background-color=00000000',
            '--window-size=640,640',
            "--screenshot=$pngPath",
            $fileUrl
        ) -WindowStyle Hidden -Wait -PassThru
        if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $pngPath)) {
            throw "Failed to rasterize $svgPath"
        }
        Convert-PngToIco `
            -PngPath $pngPath `
            -IcoPath (Join-Path $outputRoot $entry.Value) `
            -TrimTransparent:($entry.Value -eq 'cheime.ico')
        Write-Host "[OK] $($entry.Key) -> $($entry.Value)"
    }
} finally {
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
