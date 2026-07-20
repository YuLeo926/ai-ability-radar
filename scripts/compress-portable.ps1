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
if (-not (Test-Path -LiteralPath $destinationDirectory -PathType Container)) {
  throw "Portable destination directory does not exist."
}
if (Test-Path -LiteralPath $destinationPath) {
  throw "Portable destination already exists."
}
$sourceItem = Get-Item -LiteralPath $sourcePath
$destinationDirectoryItem = Get-Item -LiteralPath $destinationDirectory
if (
  ($sourceItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
  ($destinationDirectoryItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
) {
  throw "Portable compressor paths must not be reparse points."
}
Compress-Archive `
  -LiteralPath $sourcePath `
  -DestinationPath $destinationPath `
  -CompressionLevel Optimal
