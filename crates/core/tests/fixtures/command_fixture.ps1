# 명령 어댑터 테스트용: 일부러 실패하거나 멈춘다.
# 사용: command_fixture.ps1 <fail|nofile|sleep> <요청 파일>
# 한글이 들어 있으므로 UTF-8(BOM 있음)으로 저장한다.
param([string]$Mode, [string]$RequestPath)
$ErrorActionPreference = 'Stop'

switch ($Mode) {
  'fail' {
    # 오류 출력은 UTF-8 바이트로 직접 쓴다(5.1 의 기본 출력 인코딩은 시스템 코드 페이지)
    $bytes = [Text.Encoding]::UTF8.GetBytes("스크립트 오류: 제품 형식이 다름`n")
    $err = [Console]::OpenStandardError()
    $err.Write($bytes, 0, $bytes.Length)
    $err.Flush()
    exit 3
  }
  'nofile' { exit 0 }
  'sleep' { Start-Sleep -Seconds 30; exit 0 }
}
exit 9
