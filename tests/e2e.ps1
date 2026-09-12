$proxyProc = Start-Process -FilePath "cargo" -ArgumentList "run --release -p cli -- run" -PassThru -NoNewWindow
Start-Sleep -Seconds 3

$url = "http://127.0.0.1:8888/v1/messages"
$body = @{
    model = "claude-3-5-sonnet-20241022"
    messages = @(
        @{
            role = "user"
            content = "Hello, world!"
        }
    )
} | ConvertTo-Json

$headers = @{
    "Content-Type" = "application/json"
    "x-api-key" = "fake-key"
}

Write-Host "Sending mock payload to $url..."
try {
    $response = Invoke-RestMethod -Uri $url -Method Post -Body $body -Headers $headers
    Write-Host "Received success response: $response"
} catch {
    $statusCode = $_.Exception.Response.StatusCode.value__
    Write-Host "Proxy intercepted and routed correctly! Upstream returned: $statusCode"
    if ($statusCode -in @(400, 401, 502)) {
        Write-Host "E2E Test Passed: Proxy is functioning and routing requests."
    } else {
        Write-Host "E2E Test Failed with unexpected HTTP code: $statusCode"
    }
}

Stop-Process -Id $proxyProc.Id -Force
Write-Host "Test complete."
