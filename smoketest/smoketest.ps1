# Change working directory to the script's directory
Set-Location -Path $PSScriptRoot

# Generate certificates
New-Item -ItemType Directory -Path "certs" -Force | Out-Null
openssl req -new -newkey rsa:4096 -nodes `
    -keyout certs/server.key -out certs/server.csr `
    -subj "/CN=localhost"
openssl x509 -req -days 3650 -in certs/server.csr -signkey certs/server.key -out certs/server.crt

# Start Ferron in the background
$FerronProcess = Start-Process -FilePath $env:FERRON -PassThru

# Wait for Ferron to start
Start-Sleep -Seconds 5

# Perform the smoke test
$Got = curl.exe -sk https://localhost:8443/test.txt
$Expected = Get-Content -Raw "wwwroot/test.txt"

if ($Got -eq $Expected) {
    Write-Output "Test passed"
    Stop-Process -Id $FerronProcess.Id -Force
} else {
    Write-Error "Test failed"
    Stop-Process -Id $FerronProcess.Id -Force
    exit 1
}
