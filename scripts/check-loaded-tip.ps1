$output = Join-Path $env:TEMP "cheime-loaded-modules.txt"
cmd.exe /c "tasklist /m cheime-tip.dll > `"$output`" 2>&1"
