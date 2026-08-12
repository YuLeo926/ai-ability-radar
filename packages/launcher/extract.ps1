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
  throw "Archive source is unavailable."
}
if ([System.IO.Path]::GetExtension($sourcePath) -cne ".zip") {
  throw "Archive source must be a zip file."
}
if (-not (Test-Path -LiteralPath $destinationPath -PathType Container)) {
  throw "Extraction destination is unavailable."
}
$sourceItem = Get-Item -LiteralPath $sourcePath -Force
$destinationItem = Get-Item -LiteralPath $destinationPath -Force
if (
  ($sourceItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or
  ($destinationItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
) {
  throw "Extractor paths must not be reparse points."
}
if ((Get-ChildItem -LiteralPath $destinationPath -Force).Count -ne 0) {
  throw "Extraction destination must be empty."
}

$separator = [System.IO.Path]::DirectorySeparatorChar
$destinationPrefix = $destinationPath.TrimEnd($separator) + $separator
$seen = [System.Collections.Generic.HashSet[string]]::new(
  [System.StringComparer]::OrdinalIgnoreCase
)
$types = [System.Collections.Generic.Dictionary[string, string]]::new(
  [System.StringComparer]::OrdinalIgnoreCase
)
$entries = [System.Collections.Generic.List[object]]::new()
$archive = [System.IO.Compression.ZipFile]::OpenRead($sourcePath)
try {
  foreach ($entry in $archive.Entries) {
    $rawName = $entry.FullName
    if (
      [string]::IsNullOrEmpty($rawName) -or
      $rawName.StartsWith("/") -or
      $rawName.StartsWith("\") -or
      $rawName.Contains("\") -or
      ($rawName.Normalize([Text.NormalizationForm]::FormC) -cne $rawName)
    ) {
      throw "Archive member name is unsafe."
    }
    $isDirectory = $rawName.EndsWith("/")
    $entryName = if ($isDirectory) { $rawName.Substring(0, $rawName.Length - 1) } else { $rawName }
    $segments = $entryName.Split('/')
    if ($segments[0] -cne "ability-radar-portable") {
      throw "Archive member is outside the fixed root."
    }
    foreach ($segment in $segments) {
      if (
        [string]::IsNullOrEmpty($segment) -or
        $segment -eq "." -or
        $segment -eq ".." -or
        $segment.EndsWith(".") -or
        $segment.EndsWith(" ") -or
        $segment -match '[\x00-\x1f\x7f<>:"|?*]' -or
        $segment.Split('.')[0] -match '^(?i:CON|PRN|AUX|NUL|CLOCK\$|CONIN\$|CONOUT\$|COM[1-9]|LPT[1-9])$'
      ) {
        throw "Archive member component is unsafe."
      }
    }
    if (-not $seen.Add($entryName)) {
      throw "Archive member destination is duplicated."
    }
    $types.Add($entryName, $(if ($isDirectory) { "directory" } else { "file" }))
    if ($isDirectory -and ($entry.Length -ne 0 -or $entry.CompressedLength -ne 0)) {
      throw "Archive directory has payload bytes."
    }
    $attributes = [uint32]([int64]$entry.ExternalAttributes -band 0xffffffffL)
    if (
      (($attributes -band 0x0400) -ne 0) -or
      ((($attributes -shr 16) -band 0xf000) -eq 0xa000)
    ) {
      throw "Archive member is a link or reparse point."
    }
    $entryPath = [System.IO.Path]::GetFullPath(
      [System.IO.Path]::Combine($destinationPath, $entryName.Replace('/', $separator))
    )
    if (-not $entryPath.StartsWith($destinationPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
      throw "Archive member escapes the destination."
    }
    $entries.Add([pscustomobject]@{
      ArchiveEntry = $entry
      EntryName = $entryName
      EntryPath = $entryPath
      IsDirectory = $isDirectory
    })
  }
  foreach ($item in $entries) {
    $parts = $item.EntryName.Split('/')
    for ($index = 1; $index -lt $parts.Length; $index += 1) {
      $ancestor = [string]::Join('/', $parts[0..($index - 1)])
      if ($types.ContainsKey($ancestor) -and $types[$ancestor] -eq "file") {
        throw "Archive file aliases a directory."
      }
    }
  }

  foreach ($item in $entries) {
    if ($item.IsDirectory) {
      [void][System.IO.Directory]::CreateDirectory($item.EntryPath)
      continue
    }
    $parent = [System.IO.Path]::GetDirectoryName($item.EntryPath)
    [void][System.IO.Directory]::CreateDirectory($parent)
    $inputStream = $item.ArchiveEntry.Open()
    $outputStream = $null
    try {
      $outputStream = [System.IO.File]::Open(
        $item.EntryPath,
        [System.IO.FileMode]::CreateNew,
        [System.IO.FileAccess]::Write,
        [System.IO.FileShare]::None
      )
      $inputStream.CopyTo($outputStream)
      $outputStream.Flush($true)
    }
    finally {
      if ($null -ne $outputStream) { $outputStream.Dispose() }
      $inputStream.Dispose()
    }
  }
}
finally {
  $archive.Dispose()
}
