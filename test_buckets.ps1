# Test ActivityWatch bucket response
Write-Host "Fetching ActivityWatch buckets..." -ForegroundColor Yellow

try {
    $response = Invoke-WebRequest -Uri "http://localhost:5600/api/0/buckets/" -UseBasicParsing
    $buckets = $response.Content | ConvertFrom-Json
    
    Write-Host "`nBucket response:" -ForegroundColor Cyan
    Write-Host $response.Content
    
    Write-Host "`nBucket IDs:" -ForegroundColor Green
    foreach ($bucket in $buckets.PSObject.Properties) {
        Write-Host "  - $($bucket.Name)"
    }
} catch {
    Write-Host "Error: $_" -ForegroundColor Red
}