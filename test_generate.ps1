Write-Host "Testing generate function directly..." -ForegroundColor Yellow

# First check connections
Write-Host "`nChecking connections..." -ForegroundColor Cyan
$connections = curl -s http://localhost:1420/check_connections
Write-Host "Connections: $connections"

# Try to generate summary
Write-Host "`nCalling generate_hourly_summary..." -ForegroundColor Cyan
try {
    $response = Invoke-WebRequest -Uri "http://localhost:1420/generate_hourly_summary" -Method POST -UseBasicParsing
    Write-Host "Response: $($response.Content)" -ForegroundColor Green
} catch {
    Write-Host "Error: $_" -ForegroundColor Red
    Write-Host "Status: $($_.Exception.Response.StatusCode)" -ForegroundColor Red
}