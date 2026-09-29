# Print one PLAYER line per new player tape record. Silent otherwise.
param(
    [Parameter(Mandatory = $true)]
    [string]$Tape
)

$ErrorActionPreference = 'SilentlyContinue'
$ProgressPreference = 'SilentlyContinue'

$script:High = $null
$script:Offset = [int64]0
$script:Seen = $false
$script:Pending = New-Object System.Text.StringBuilder

function Write-Out([string]$Line) {
    [Console]::Out.WriteLine($Line)
    [Console]::Out.Flush()
}

function Emit-Player([string]$Line) {
    try {
        $obj = $Line | ConvertFrom-Json
    } catch {
        return
    }
    $from = [string]$obj.from
    if (-not $from.StartsWith('player:')) {
        return
    }
    if ($null -eq $obj.seq) {
        return
    }
    $seq = [int64]$obj.seq
    if ($null -ne $script:High -and $seq -le $script:High) {
        return
    }
    $text = ([string]$obj.text) -replace '[\r\n]+', ' '
    $parts = New-Object System.Collections.Generic.List[string]
    $parts.Add('PLAYER')
    $parts.Add("seq=$seq")
    $parts.Add("from=$from")
    $parts.Add("address=$([string]$obj.address)")
    if ($obj.place) { $parts.Add("place=$($obj.place)") }
    if ($obj.actor) { $parts.Add("actor=$($obj.actor)") }
    $parts.Add("text=$text")
    Write-Out ($parts -join ' ')
    $script:High = $seq
}

function Read-New([string]$Path) {
    $stream = $null
    try {
        $stream = [System.IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite')
        $len = $stream.Length
        if ($len -lt $script:Offset) {
            $script:Offset = [int64]0
            $script:Pending.Length = 0
        }
        if ($len -le $script:Offset) {
            return
        }
        $stream.Seek($script:Offset, 'Begin') | Out-Null
        $left = $len - $script:Offset
        $bucket = New-Object System.IO.MemoryStream
        $buf = New-Object byte[] 8192
        while ($left -gt 0) {
            $want = [int][Math]::Min(8192, $left)
            $n = $stream.Read($buf, 0, $want)
            if ($n -le 0) { break }
            $bucket.Write($buf, 0, $n)
            $left -= $n
        }
        $script:Offset = $stream.Position
        $chunk = [System.Text.Encoding]::UTF8.GetString($bucket.ToArray())
        [void]$script:Pending.Append($chunk)
        $all = $script:Pending.ToString()
        $cut = $all.LastIndexOf("`n")
        if ($cut -lt 0) { return }
        $done = $all.Substring(0, $cut)
        $rest = $all.Substring($cut + 1)
        $script:Pending.Length = 0
        [void]$script:Pending.Append($rest)
        foreach ($row in ($done -split "`n")) {
            $row = $row.TrimEnd("`r")
            if ($row.Length -gt 0) {
                Emit-Player $row
            }
        }
    } catch {
        return
    } finally {
        if ($null -ne $stream) { $stream.Dispose() }
    }
}

if (Test-Path -LiteralPath $Tape) {
    $script:Offset = [int64](Get-Item -LiteralPath $Tape).Length
    $script:Seen = $true
}

while ($true) {
    if (-not (Test-Path -LiteralPath $Tape)) {
        if ($script:Seen) {
            $script:Seen = $false
            $script:Offset = [int64]0
            $script:High = $null
            $script:Pending.Length = 0
            Write-Out 'TAPE reset'
        }
        Start-Sleep -Milliseconds 500
        continue
    }
    $script:Seen = $true
    Read-New $Tape
    Start-Sleep -Milliseconds 500
}
