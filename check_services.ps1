# Check if ActivityWatch is running
Write-Host "Checking ActivityWatch..." -ForegroundColor Yellow
try {
    $response = Invoke-WebRequest -Uri "http://localhost:5600/api/0/info" -UseBasicParsing -TimeoutSec 2
    if ($response.StatusCode -eq 200) {
        Write-Host "✓ ActivityWatch is running on port 5600" -ForegroundColor Green
    }
} catch {
    Write-Host "✗ ActivityWatch is NOT running on port 5600" -ForegroundColor Red
    Write-Host "  Please start ActivityWatch first" -ForegroundColor Yellow
}

Write-Host ""

# Check if Ollama is running
Write-Host "Checking Ollama..." -ForegroundColor Yellow
try {
    $response = Invoke-WebRequest -Uri "http://localhost:11434/api/tags" -UseBasicParsing -TimeoutSec 2
    if ($response.StatusCode -eq 200) {
        Write-Host "✓ Ollama is running on port 11434" -ForegroundColor Green
    }
} catch {
    Write-Host "✗ Ollama is NOT running on port 11434" -ForegroundColor Red
    Write-Host "  Please run 'ollama serve' first" -ForegroundColor Yellow
}