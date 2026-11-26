## Installing

Download `NetLantern-__VERSION__-setup.exe` and run it. Windows 11 only. No administrator rights are needed.

> Net Lantern isn't code-signed yet, so Windows SmartScreen may say "Windows protected your PC" the first time you run the installer. Check the file against SHA256SUMS.txt (below), then click **More info** and **Run anyway**.

### Checking the checksum

In PowerShell, run:

```powershell
Get-FileHash .\NetLantern-__VERSION__-setup.exe -Algorithm SHA256
```

and compare the hash with the matching line in `SHA256SUMS.txt` (the letter case doesn't matter).
