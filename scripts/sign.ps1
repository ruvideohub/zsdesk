# Signs the built executables.
#
# A real code-signing certificate is the only thing that removes SmartScreen's
# "unknown publisher" warning. Until one exists, this script signs with a self-signed
# certificate so every shipped binary carries a valid Authenticode signature that can be
# inspected with `Get-AuthenticodeSignature` (and trusted by hand if you want).
#
# For a real certificate, put the PFX in repository secrets as base64 and set
# `ZSDesk_SIGN_PFX_B64` and `ZSDesk_SIGN_PFX_PASSWORD`; the script then uses it instead
# of generating a throwaway self-signed one.
#
#     ./scripts/sign.ps1 -Path dist

[CmdletBinding()]
param(
    [string]$Path = 'dist',
    [string]$Subject = 'CN=ZSDesk Self-Signed, O=ZSDesk, C=RU'
)

$ErrorActionPreference = 'Stop'

function Find-SignTool {
    $candidates = @(
        (Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe' -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending | Select-Object -First 1),
        (Get-ChildItem 'C:\Program Files\Windows Kits\10\bin\*\x64\signtool.exe' -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending | Select-Object -First 1)
    ) | Where-Object { $_ }
    if ($candidates) { return $candidates[0].FullName }
    $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    return $null
}

$signTool = Find-SignTool
if (-not $signTool) {
    Write-Warning 'signtool.exe was not found; the executables stay unsigned.'
    exit 0
}

$executables = @(Get-ChildItem -LiteralPath $Path -Filter '*.exe' -ErrorAction SilentlyContinue)
if ($executables.Count -eq 0) {
    Write-Warning "No executables in $Path; nothing to sign."
    exit 0
}

$pfx = Join-Path ([IO.Path]::GetTempPath()) 'zsdesk-signing.pfx'
$password = 'zsdesk-self-signed'

try {
    if ($env:ZSDesk_SIGN_PFX_B64) {
        Write-Host 'Signing with the certificate from the repository secrets.'
        [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:ZSDesk_SIGN_PFX_B64))
        if ($env:ZSDesk_SIGN_PFX_PASSWORD) { $password = $env:ZSDesk_SIGN_PFX_PASSWORD }
    } else {
        Write-Host 'Signing with a throwaway self-signed certificate (no secret configured).'
        $cert = New-SelfSignedCertificate `
            -Type CodeSigningCert `
            -Subject $Subject `
            -KeyUsage DigitalSignature `
            -CertStoreLocation 'Cert:\CurrentUser\My' `
            -NotAfter (Get-Date).AddYears(3)
        $secure = ConvertTo-SecureString -String $password -Force -AsPlainText
        Export-PfxCertificate -Cert $cert -FilePath $pfx -Password $secure | Out-Null
    }

    foreach ($exe in $executables) {
        & $signTool sign /f $pfx /p $password /fd sha256 /a $exe.FullName | Out-Host
        if ($LASTEXITCODE -ne 0) { throw "signtool failed for $($exe.Name)" }
        $check = Get-AuthenticodeSignature -LiteralPath $exe.FullName
        Write-Host ("  {0}: {1}" -f $exe.Name, $check.Status)
    }
}
catch {
    Write-Warning "Signing failed: $($_.Exception.Message)"
}
finally {
    Remove-Item -LiteralPath $pfx -Force -ErrorAction SilentlyContinue
}
