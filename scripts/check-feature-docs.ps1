Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$readmePath = Join-Path $repoRoot "README.md"
$readmeEnPath = Join-Path $repoRoot "README.en.md"
$featureDir = Join-Path $repoRoot "docs\features"
$featureFiles = @(Get-ChildItem -LiteralPath $featureDir -Filter "*.md" -File)
$errors = [System.Collections.Generic.List[string]]::new()

$expectedFeatures = @($featureFiles | ForEach-Object { $_.Name } | Sort-Object -Unique)
$listedFeatures = @()

foreach ($readmeFile in @($readmePath, $readmeEnPath)) {
    $readme = Get-Content -Raw -Encoding UTF8 -LiteralPath $readmeFile
    $listed = @(
        [regex]::Matches($readme, "\(docs/features/([^)]+\.md)\)") |
            ForEach-Object { $_.Groups[1].Value } |
            Sort-Object -Unique
    )
    foreach ($missing in @($expectedFeatures | Where-Object { $_ -notin $listed })) {
        $errors.Add("$(Split-Path -Leaf $readmeFile) is missing $missing")
    }
    foreach ($stale in @($listed | Where-Object { $_ -notin $expectedFeatures })) {
        $errors.Add("$(Split-Path -Leaf $readmeFile) links to missing feature file $stale")
    }
    if ($readmeFile -eq $readmePath) { $listedFeatures = $listed }
}

foreach ($markdownPath in @($readmePath, $readmeEnPath) + @($featureFiles | ForEach-Object { $_.FullName })) {
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

    if ($markdownPath -like "*\docs\features\*") {
        $images = @(
            [regex]::Matches($markdown, 'src="\.\./images/([^"]+)"') |
                ForEach-Object { $_.Groups[1].Value }
        )

        if ($images.Count -eq 0) {
            $errors.Add("$markdownPath has no screenshot")
        }
        foreach ($image in $images) {
            if (-not (Test-Path -LiteralPath (Join-Path $repoRoot "docs\images\$image"))) {
                $errors.Add("$markdownPath references missing image $image")
            }
        }
        if (-not ($images | Where-Object { $_ -notin @("main.png", "settings.png") })) {
            $errors.Add("$markdownPath only uses generic screenshots")
        }
    }
}

if ($errors.Count) {
    $errors | ForEach-Object { Write-Error $_ }
    exit 1
}

Write-Output "feature docs: $($featureFiles.Count); README links: $($listedFeatures.Count); local references: checked"
