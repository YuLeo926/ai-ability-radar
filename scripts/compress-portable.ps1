param(
  [Parameter(Mandatory = $true)]
  [string]$Source,
  [Parameter(Mandatory = $true)]
  [string]$Destination
)

$ErrorActionPreference = "Stop"
$sourcePath = [System.IO.Path]::GetFullPath($Source)
$destinationPath = [System.IO.Path]::GetFullPath($Destination)
if (-not (Test-Path -LiteralPath $sourcePath -PathType Container)) {
  throw "Portable source directory does not exist."
}
if ([System.IO.Path]::GetExtension($destinationPath) -cne ".zip") {
  throw "Portable destination must be a .zip file."
}
$destinationDirectory = Split-Path -Parent $destinationPath
New-Item -ItemType Directory -Force -Path $destinationDirectory | Out-Null
Compress-Archive `
  -LiteralPath $sourcePath `
  -DestinationPath $destinationPath `
  -CompressionLevel Optimal `
  -Force
