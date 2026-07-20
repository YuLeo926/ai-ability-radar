param(
  [Parameter(Mandatory = $true)]
  [string]$Source,
  [Parameter(Mandatory = $true)]
  [string]$Destination
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.IO.Compression.FileSystem
$sourcePath = [System.IO.Path]::GetFullPath($Source)
$destinationPath = [System.IO.Path]::GetFullPath($Destination)
if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
  throw "Portable archive source does not exist."
}
if ([System.IO.Path]::GetExtension($sourcePath) -cne ".zip") {
  throw "Portable archive source must be a .zip file."
}
if (-not (Test-Path -LiteralPath $destinationPath -PathType Container)) {
  throw "Portable verification directory does not exist."
}
$sourceItem = Get-Item -LiteralPath $sourcePath
$destinationItem = Get-Item -LiteralPath $destinationPath
if (
  ($sourceItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
  ($destinationItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
) {
  throw "Portable extractor paths must not be reparse points."
}
if ((Get-ChildItem -LiteralPath $destinationPath).Count -ne 0) {
  throw "Portable verification directory must be empty."
}
$separator = [System.IO.Path]::DirectorySeparatorChar
$destinationPrefix = $destinationPath.TrimEnd($separator) + $separator
$seen = [System.Collections.Generic.HashSet[string]]::new(
  [System.StringComparer]::OrdinalIgnoreCase
)
$archive = [System.IO.Compression.ZipFile]::OpenRead($sourcePath)
try {
  foreach ($entry in $archive.Entries) {
    $entryName = $entry.FullName.Replace("\", "/").TrimEnd("/")
    if ([string]::IsNullOrWhiteSpace($entryName)) {
      throw "Portable archive contains an empty entry name."
    }
    if (-not $seen.Add($entryName)) {
      throw "Portable archive contains a duplicate entry."
    }
    $entryPath = [System.IO.Path]::GetFullPath(
      [System.IO.Path]::Combine(
        $destinationPath,
        $entryName.Replace("/", $separator)
      )
    )
    if (-not $entryPath.StartsWith(
      $destinationPrefix,
      [System.StringComparison]::OrdinalIgnoreCase
    )) {
      throw "Portable archive entry escapes the verification directory."
    }
  }
}
finally {
  $archive.Dispose()
}
Expand-Archive `
  -LiteralPath $sourcePath `
  -DestinationPath $destinationPath
