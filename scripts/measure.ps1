param(
    [int]$Subscriptions = 1,
    [int]$WarmupSeconds = 3,
    [int]$SampleSeconds = 5,
    [switch]$IncludeUreq
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("wintfy-rs-measure-" + [System.Guid]::NewGuid().ToString("N"))
$server = $null
$appProcess = $null

if ($Subscriptions -lt 1) {
    throw "Subscriptions must be at least 1"
}
if ($WarmupSeconds -lt 1) {
    throw "WarmupSeconds must be at least 1"
}
if ($SampleSeconds -lt 1) {
    throw "SampleSeconds must be at least 1"
}

$existing = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -like "wintfy-rs*" }
if ($existing) {
    $ids = ($existing | ForEach-Object { "$($_.ProcessName)($($_.Id))" }) -join ", "
    throw "stop existing wintfy-rs processes before measuring: $ids"
}

function Stop-ProcessIfRunning {
    param([System.Diagnostics.Process]$Process)

    if ($null -eq $Process) {
        return
    }
    try {
        if (-not $Process.HasExited) {
            Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
            $Process.WaitForExit(5000) | Out-Null
        }
    } catch {
    }
}

function Stop-Server {
    param([System.Diagnostics.Process]$Process)

    Stop-ProcessIfRunning $Process
}

function Get-FreePort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try {
        return $listener.LocalEndpoint.Port
    } finally {
        $listener.Stop()
    }
}

function Start-MockServer {
    param(
        [int]$Port,
        [string]$ConnectionFile,
        [int]$KeepaliveSeconds
    )

    $script = @'
param(
    [int]$Port,
    [string]$ConnectionFile,
    [int]$KeepaliveSeconds
)

$ErrorActionPreference = "Stop"

function Write-Chunk {
    param(
        [System.Net.Sockets.NetworkStream]$Stream,
        [string]$Text
    )

    $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
    $head = [System.Text.Encoding]::ASCII.GetBytes(("{0:x}`r`n" -f $bytes.Length))
    $tail = [System.Text.Encoding]::ASCII.GetBytes("`r`n")
    $Stream.Write($head, 0, $head.Length)
    $Stream.Write($bytes, 0, $bytes.Length)
    $Stream.Write($tail, 0, $tail.Length)
    $Stream.Flush()
}

$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $Port)
$listener.Start()
try {
    while ($true) {
        $client = $listener.AcceptTcpClient()
        Add-Content -LiteralPath $ConnectionFile -Value "1"
        $null = [System.Threading.ThreadPool]::QueueUserWorkItem({
            param($State)

            $tcp = [System.Net.Sockets.TcpClient]$State.Client
            $interval = [int]$State.KeepaliveSeconds
            try {
                $stream = $tcp.GetStream()
                $stream.ReadTimeout = 2000
                $buffer = [byte[]]::new(1024)
                $request = [System.Collections.Generic.List[byte]]::new()
                while ($true) {
                    $read = $stream.Read($buffer, 0, $buffer.Length)
                    if ($read -le 0) {
                        return
                    }
                    for ($i = 0; $i -lt $read; $i++) {
                        $request.Add($buffer[$i])
                    }
                    if ($request.Count -ge 4) {
                        $n = $request.Count
                        if ($request[$n - 4] -eq 13 -and $request[$n - 3] -eq 10 -and $request[$n - 2] -eq 13 -and $request[$n - 1] -eq 10) {
                            break
                        }
                    }
                }

                $headers = [System.Text.Encoding]::ASCII.GetBytes("HTTP/1.1 200 OK`r`nContent-Type: application/x-ndjson`r`nTransfer-Encoding: chunked`r`nConnection: close`r`n`r`n")
                $stream.Write($headers, 0, $headers.Length)
                $stream.Flush()
                Write-Chunk $stream "{`"event`":`"open`"}`n"
                while ($tcp.Connected) {
                    Start-Sleep -Seconds $interval
                    Write-Chunk $stream "{`"event`":`"keepalive`"}`n"
                }
            } catch {
            } finally {
                $tcp.Close()
            }
        }, @{
            Client = $client
            KeepaliveSeconds = $KeepaliveSeconds
        })
    }
} finally {
    $listener.Stop()
}
'@

    $serverScript = Join-Path $tempRoot ("mock-server-{0}.ps1" -f $Port)
    Set-Content -LiteralPath $serverScript -Value $script -Encoding UTF8
    $args = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", $serverScript,
        "-Port", $Port,
        "-ConnectionFile", $ConnectionFile,
        "-KeepaliveSeconds", $KeepaliveSeconds
    )
    return Start-Process -FilePath "powershell" -ArgumentList $args -WindowStyle Hidden -PassThru
}

function Wait-ForPort {
    param(
        [int]$Port,
        [int]$TimeoutMilliseconds = 5000
    )

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMilliseconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $client = [System.Net.Sockets.TcpClient]::new()
        try {
            $connect = $client.BeginConnect("127.0.0.1", $Port, $null, $null)
            if ($connect.AsyncWaitHandle.WaitOne(200)) {
                $client.EndConnect($connect)
                return
            }
        } catch {
        } finally {
            $client.Close()
        }
        Start-Sleep -Milliseconds 100
    }
    throw "mock server did not open port $Port"
}

function New-MeasureConfig {
    param(
        [string]$Path,
        [int]$Port,
        [int]$SubscriptionCount
    )

    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add("[app]")
    $lines.Add('log_level = "warn"')
    $lines.Add("show_connection_status_toast = false")
    $lines.Add("")
    $lines.Add("[network]")
    $lines.Add("reconnect_initial_seconds = 60")
    $lines.Add("reconnect_max_seconds = 60")
    $lines.Add("reconnect_jitter = false")
    $lines.Add("line_max_bytes = 1048576")
    $lines.Add("")
    $lines.Add("[security]")
    $lines.Add("allow_http = true")
    $lines.Add('allow_url_schemes = ["http", "https"]')
    $lines.Add("allow_dangerous_url_schemes = false")
    $lines.Add("")

    for ($i = 1; $i -le $SubscriptionCount; $i++) {
        $lines.Add("[[subscriptions]]")
        $lines.Add(('name = "local-{0}"' -f $i))
        $lines.Add(('server = "http://127.0.0.1:{0}"' -f $Port))
        $lines.Add(('topics = ["measure-{0}"]' -f $i))
        $lines.Add("")
    }

    Set-Content -LiteralPath $Path -Value $lines -Encoding UTF8
}

function Get-ProcessSnapshot {
    param([int]$Id)

    $proc = Get-Process -Id $Id -ErrorAction Stop
    [pscustomobject]@{
        PrivateBytes = $proc.PrivateMemorySize64
        WorkingSet = $proc.WorkingSet64
        Threads = $proc.Threads.Count
        Handles = $proc.HandleCount
        CpuSeconds = if ($null -eq $proc.CPU) { 0.0 } else { [double]$proc.CPU }
    }
}

function Invoke-MeasureRun {
    param(
        [string]$Name,
        [string]$BuildArgs,
        [string]$SourceExe,
        [int]$SubscriptionCount
    )

    Write-Host "Building $Name..."
    if ($BuildArgs.Length -eq 0) {
        cargo build --release --manifest-path (Join-Path $root "Cargo.toml")
    } else {
        $parts = $BuildArgs -split " "
        cargo build @parts --manifest-path (Join-Path $root "Cargo.toml")
    }

    if (-not (Test-Path -LiteralPath $SourceExe)) {
        throw "missing executable: $SourceExe"
    }

    $runDir = Join-Path $tempRoot $Name
    New-Item -ItemType Directory -Path $runDir -Force | Out-Null
    $exe = Join-Path $runDir ("wintfy-rs-{0}.exe" -f $Name)
    Copy-Item -LiteralPath $SourceExe -Destination $exe -Force

    $port = Get-FreePort
    $connections = Join-Path $runDir "connections.txt"
    New-Item -ItemType File -Path $connections -Force | Out-Null
    $config = Join-Path $runDir "config.toml"
    New-MeasureConfig -Path $config -Port $port -SubscriptionCount $SubscriptionCount

    $script:server = Start-MockServer -Port $port -ConnectionFile $connections -KeepaliveSeconds 15
    Wait-ForPort -Port $port

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $exe
    $startInfo.ArgumentList.Add("--config")
    $startInfo.ArgumentList.Add($config)
    $startInfo.WorkingDirectory = $runDir
    $startInfo.UseShellExecute = $false
    $script:appProcess = [System.Diagnostics.Process]::Start($startInfo)

    try {
        Start-Sleep -Seconds $WarmupSeconds
        if ($script:appProcess.HasExited) {
            throw "$Name process exited during warmup with code $($script:appProcess.ExitCode)"
        }
        $before = Get-ProcessSnapshot -Id $script:appProcess.Id
        Start-Sleep -Seconds $SampleSeconds
        if ($script:appProcess.HasExited) {
            throw "$Name process exited during sampling with code $($script:appProcess.ExitCode)"
        }
        $after = Get-ProcessSnapshot -Id $script:appProcess.Id

        $connectionCount = 0
        if (Test-Path -LiteralPath $connections) {
            $accepted = (Get-Content -LiteralPath $connections -ErrorAction SilentlyContinue | Measure-Object).Count
            $connectionCount = [Math]::Max(0, $accepted - 1)
        }

        return [pscustomobject]@{
            Backend = $Name
            Subscriptions = $SubscriptionCount
            ExeBytes = (Get-Item -LiteralPath $exe).Length
            PrivateBytes = $after.PrivateBytes
            WorkingSetBytes = $after.WorkingSet
            Threads = $after.Threads
            Handles = $after.Handles
            IdleCpuSeconds = [Math]::Max(0.0, $after.CpuSeconds - $before.CpuSeconds)
            SampleSeconds = $SampleSeconds
            Connections = $connectionCount
        }
    } finally {
        Stop-ProcessIfRunning $script:appProcess
        $script:appProcess = $null
        Stop-Server $script:server
        $script:server = $null
    }
}

try {
    New-Item -ItemType Directory -Path $tempRoot -Force | Out-Null

    $runs = @(
        @{
            Name = "native-tls"
            BuildArgs = "--release"
            SourceExe = Join-Path $root "target\release\wintfy-rs.exe"
        },
        @{
            Name = "winhttp"
            BuildArgs = "--release --no-default-features --features toml-config,native-toast,winhttp"
            SourceExe = Join-Path $root "target\release\wintfy-rs.exe"
        }
    )

    if ($IncludeUreq) {
        $runs += @{
            Name = "ureq"
            BuildArgs = "--release --no-default-features --features toml-config,native-toast,ureq-client"
            SourceExe = Join-Path $root "target\release\wintfy-rs.exe"
        }
    }

    $results = foreach ($run in $runs) {
        Invoke-MeasureRun `
            -Name $run.Name `
            -BuildArgs $run.BuildArgs `
            -SourceExe $run.SourceExe `
            -SubscriptionCount $Subscriptions
    }

    $results | Format-Table `
        Backend,
        Subscriptions,
        ExeBytes,
        @{Label = "PrivateMB"; Expression = { "{0:N2}" -f ($_.PrivateBytes / 1MB) }},
        @{Label = "WorkingSetMB"; Expression = { "{0:N2}" -f ($_.WorkingSetBytes / 1MB) }},
        Threads,
        Handles,
        @{Label = "IdleCpuSec"; Expression = { "{0:N3}" -f $_.IdleCpuSeconds }},
        Connections `
        -AutoSize
} finally {
    Stop-ProcessIfRunning $appProcess
    Stop-Server $server
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}
