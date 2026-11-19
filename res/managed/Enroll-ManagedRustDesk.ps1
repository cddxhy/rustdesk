[CmdletBinding()]
param([string] $EnrollmentTokenFile)

$ErrorActionPreference = 'Stop'
$principal = [Security.Principal.WindowsPrincipal]::new(
    [Security.Principal.WindowsIdentity]::GetCurrent()
)
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run enrollment from an elevated Windows PowerShell session.'
}
if (![Environment]::Is64BitProcess) {
    throw 'Run enrollment in 64-bit Windows PowerShell.'
}

$state = Join-Path $env:ProgramFiles 'RustDesk-Managed-State'
$acl = [Security.AccessControl.DirectorySecurity]::new()
$acl.SetAccessRuleProtection($true, $false)
foreach ($sid in 'S-1-5-18', 'S-1-5-32-544') {
    $rule = [Security.AccessControl.FileSystemAccessRule]::new(
        [Security.Principal.SecurityIdentifier]::new($sid),
        'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow'
    )
    $acl.AddAccessRule($rule)
}
if (Test-Path -LiteralPath $state) {
    $item = Get-Item -LiteralPath $state -Force
    if (!$item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'The RustDesk enrollment state directory is unsafe.'
    }
} else {
    [IO.Directory]::CreateDirectory($state, $acl) | Out-Null
}
Set-Acl -LiteralPath $state -AclObject $acl

$registration = Join-Path $state 'enrollment.token'
if ((Test-Path -LiteralPath $registration) -or
    (Test-Path -LiteralPath (Join-Path $state 'device.json'))) {
    throw 'This device already has an enrollment or enrollment in progress.'
}

if ($EnrollmentTokenFile) {
    $token = [IO.File]::ReadAllText((Resolve-Path -LiteralPath $EnrollmentTokenFile).Path).Trim()
} else {
    $secureToken = Read-Host 'One-time enrollment token' -AsSecureString
    $buffer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureToken)
    try {
        $token = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($buffer).Trim()
    } finally {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($buffer)
        $secureToken.Dispose()
    }
}
if ($token -cnotmatch '^[A-Za-z0-9._~-]{16,4096}$') {
    throw 'Invalid enrollment token format.'
}

$temporary = Join-Path $state ([Guid]::NewGuid().ToString() + '.tmp')
try {
    [IO.File]::WriteAllText($temporary, $token, [Text.UTF8Encoding]::new($false))
    [IO.File]::Move($temporary, $registration)
} finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    $token = $null
}

Start-Service -Name 'RustDesk'
Write-Output 'Enrollment queued. Check the device status in the management API.'
