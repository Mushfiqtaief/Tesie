# Probes the real context window of OpenCode's free DeepSeek model by sending
# progressively larger prompts and reporting the upstream verdict at each size.
param(
    [string]$Model = "deepseek-v4.1-flash-free",
    [int[]]$TargetTokens = @(150000, 220000, 400000, 700000),
    [string]$BaseUrl = "http://127.0.0.1:20129/v1",
    [string]$OutFile = "D:\Apps\TesRoute\.data\deepseek_context_probe.log"
)

$ErrorActionPreference = "Stop"
$cfg = Get-Content "$env:USERPROFILE\.config\opencode\opencode.json" -Raw | ConvertFrom-Json
$key = $cfg.provider.tesie.options.apiKey
$headers = @{
    Authorization            = "Bearer $key"
    "Content-Type"           = "application/json"
    "x-tesie-token-saver"    = "off"
}

# English prose approximates ~4 chars/token for these models; we report both the
# requested target and the actual byte size so the measurement is auditable.
$chunk = "The quick brown fox jumps over the lazy dog near the riverbank at dawn. "
$chunkTokens = [math]::Round($chunk.Length / 4.0, 2)

function New-Payload {
    param([int]$Tokens)
    $reps = [math]::Ceiling(($Tokens * 4.0) / $chunk.Length)
    $sb = [System.Text.StringBuilder]::new($Tokens * 4)
    for ($i = 0; $i -lt $reps; $i++) {
        [void]$sb.Append($chunk)
        if (($i % 40) -eq 39) { [void]$sb.Append("`nLine $i of the marker sequence.`n") }
    }
    [void]$sb.Append("`nReply with exactly one word: OK")
    return $sb.ToString()
}

function Invoke-Probe {
    param([int]$Tokens)
    $text = New-Payload -Tokens $Tokens
    $bytes = [System.Text.Encoding]::UTF8.GetByteCount($text)
    $body = @{
        model      = $Model
        stream     = $false
        max_tokens = 8
        messages   = @(@{ role = "user"; content = $text })
    } | ConvertTo-Json -Depth 8 -Compress

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $status = $null; $verdict = $null; $raw = $null
    try {
        $resp = Invoke-WebRequest -Uri "$BaseUrl/chat/completions" -Method Post `
            -Headers $headers -Body $body -TimeoutSec 600 -UseBasicParsing
        $status = [int]$resp.StatusCode
        $raw = $resp.Content
    } catch {
        $ex = $_.Exception
        if ($ex.Response) {
            $status = [int]$ex.Response.StatusCode
            $reader = New-Object System.IO.StreamReader($ex.Response.GetResponseStream())
            $raw = $reader.ReadToEnd()
        } else {
            $status = -1
            $raw = $ex.Message
        }
    }
    $sw.Stop()

    if ($raw) {
        try {
            $j = $raw | ConvertFrom-Json
            if ($j.error) {
                $verdict = "REJECTED: " + $j.error.code + " | " + $j.error.message
            } elseif ($j.choices) {
                $verdict = "ACCEPTED: usage.prompt_tokens=" + $j.usage.prompt_tokens
            } else {
                $verdict = "UNKNOWN: " + ($raw.Substring(0, [Math]::Min(300, $raw.Length)))
            }
        } catch {
            $verdict = "UNPARSEABLE: " + ($raw.Substring(0, [Math]::Min(300, $raw.Length)))
        }
    }

    [pscustomobject]@{
        target_tokens = $Tokens
        bytes         = $bytes
        approx_tokens = [math]::Round($bytes / 4.0)
        status        = $status
        elapsed_s     = [math]::Round($sw.Elapsed.TotalSeconds, 1)
        verdict       = $verdict
    } | Tee-Object -FilePath $OutFile -Append
}

"=== deepseek context probe: model=$Model started $(Get-Date -Format o) ===" | Tee-Object -FilePath $OutFile
foreach ($t in $TargetTokens) {
    Write-Output "--- probing ~$t tokens ---"
    Invoke-Probe -Tokens $t
}
"=== finished $(Get-Date -Format o) ===" | Tee-Object -FilePath $OutFile
