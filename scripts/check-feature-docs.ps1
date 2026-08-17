Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$readmePath = Join-Path $repoRoot "README.md"
$featureDir = Join-Path $repoRoot "docs\features"
$featureFiles = @(Get-ChildItem -LiteralPath $featureDir -Filter "*.md" -File)
$readme = Get-Content -Raw -Encoding UTF8 -LiteralPath $readmePath
$errors = [System.Collections.Generic.List[string]]::new()

$listedFeatures = @(
    [regex]::Matches($readme, "\(docs/features/([^)]+\.md)\)") |
        ForEach-Object { $_.Groups[1].Value } |
        Sort-Object -Unique
)
$expectedFeatures = @($featureFiles | ForEach-Object { $_.Name } | Sort-Object -Unique)

foreach ($missing in @($expectedFeatures | Where-Object { $_ -notin $listedFeatures })) {
    $errors.Add("README.md is missing $missing")
}
foreach ($stale in @($listedFeatures | Where-Object { $_ -notin $expectedFeatures })) {
    $errors.Add("README.md links to missing feature file $stale")
}

foreach ($markdownPath in @($readmePath) + @($featureFiles | ForEach-Object { $_.FullName })) {
    $markdown = Get-Content -Raw -Encoding UTF8 -LiteralPath $markdownPath
    $baseDir = Split-Path -Parent $markdownPath

    foreach ($match in [regex]::Matches($markdown, "\]\(([^)]+)\)")) {
        $target = $match.Groups[1].Value.Split("#")[0].Trim()
        if (-not $target -or $target -match "^https?://") { continue }
        $resolved = Join-Path $baseDir $target
        if (-not (Test-Path -LiteralPath $resolved)) {
            $errors.Add("$markdownPath -> $target")
        }
    }

    if ($markdownPath -like "*\docs\features\*" -and $markdown -notmatch 'src="\.\./images/') {
        $errors.Add("$markdownPath has no screenshot")
    }
}

if ($errors.Count) {
    $errors | ForEach-Object { Write-Error $_ }
    exit 1
}

Write-Output "feature docs: $($featureFiles.Count); README links: $($listedFeatures.Count); local references: checked"
