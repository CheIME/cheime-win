dism /online /norestart /add-package:"%SystemRoot%\servicing\Packages\Microsoft-Hyper-V-*.mum"
dism /online /enable-feature /featurename:Microsoft-Hyper-V-All /all /norestart