# 명령 어댑터 예제(Windows PowerShell 5.1 이상): 요청 JSON 을 받아 CSV(키 열 + 언어 열)를 쓰고 읽는다.
# 설정 예:
#   내보내기 명령   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/csv_adapter.ps1 export {request}
#   역가져오기 명령 powershell -NoProfile -ExecutionPolicy Bypass -File scripts/csv_adapter.ps1 import {request}
# 요청의 options.key_header 로 키 열 머리글을 바꿀 수 있다(기본 Key).
#
# 한글이 들어 있으므로 이 파일은 UTF-8(BOM 있음)으로 저장한다. 5.1 은 BOM 이 없으면 시스템 코드 페이지로 읽는다.
param(
  [Parameter(Mandatory)][string]$Action,
  [Parameter(Mandatory)][string]$RequestPath
)
$ErrorActionPreference = 'Stop'

$req = Get-Content -LiteralPath $RequestPath -Raw -Encoding UTF8 | ConvertFrom-Json
$keyHeader = if ($req.options.key_header) { [string]$req.options.key_header } else { 'Key' }

switch ($Action) {
  'export' {
    # 요청의 rows 를 CSV 로 쓴다. 값이 없는 언어는 빈 칸
    $rows = foreach ($r in $req.rows) {
      $o = [ordered]@{ $keyHeader = $r.external_key }
      foreach ($l in $req.locales) { $o[$l] = [string]$r.values.$l }
      [pscustomobject]$o
    }
    $rows | Export-Csv -LiteralPath $req.path -NoTypeInformation -Encoding UTF8
  }
  'import' {
    # 제품 파일(req.path)을 읽어 response_path 에 {"rows": [...]} 를 쓴다
    $out = foreach ($rec in Import-Csv -LiteralPath $req.path -Encoding UTF8) {
      $key = ([string]$rec.$keyHeader).Trim()
      if ($key -eq '') { continue }
      $values = [ordered]@{}
      foreach ($l in $req.locales) {
        if ($rec.PSObject.Properties.Name -contains $l) { $values[$l] = [string]$rec.$l }
      }
      [pscustomobject]@{ external_key = $key; values = $values }
    }
    $json = ConvertTo-Json -InputObject @{ rows = @($out) } -Depth 5
    Set-Content -LiteralPath $req.response_path -Value $json -Encoding UTF8
  }
  default {
    [Console]::Error.WriteLine("unknown action: $Action")
    exit 2
  }
}
