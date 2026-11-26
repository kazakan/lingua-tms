# 명령 어댑터 (`command`) — 프로토콜 1

제품 형식 파일을 TMS 가 직접 쓰지 않고, 사용자가 지정한 프로그램(파이썬 스크립트, PowerShell, exe, 배치 파일 등)에
맡기는 내보내기 어댑터다. TMS 는 요청 JSON 을 넘기고 종료 코드와 결과 파일만 본다.

- TMS 는 스크립트를 저장하지 않고 파이썬 같은 런타임도 넣지 않는다. 명령이 가리키는 프로그램은 사용자가 준비한다.
- 검증 게이트, 제품 키 제안·고정, 원문으로 채우기, 변경 이력 기록은 지금처럼 TMS 가 한다. 프로그램은 파일 형식만 맡는다.
- 예제: [`examples/command-adapter`](../examples/command-adapter) (CSV, 파이썬판과 PowerShell 판).

## 설정

내보내기 탭에서 포맷 어댑터를 `command`로 고르면 아래 설정을 입력한다(대상 설정은 .tms 에 저장되고 변경 이력이 남는다).

| 필드 | 뜻 |
|---|---|
| `export_command` | 내보내기 명령. `{request}` 자리가 반드시 있어야 한다. 예: `python scripts/product_a.py export {request}` |
| `import_command` | 역가져오기 명령. 비우면 이 대상은 역가져오기(가져오기 탭의 "제품 형식 파일")를 지원하지 않는다 |
| `locales` | 내보낼 언어. 이 순서대로 요청에 들어간다 |
| `file_extension` | 제품 파일 확장자(점 없이). 파일 대화상자에 쓴다 |
| `timeout_secs` | 시간 제한(1~3600초, 기본 60). 넘으면 프로그램을 끄고 실패로 본다 |
| `options` | JSON 객체. TMS 는 해석하지 않고 요청에 그대로 넘긴다(제품별 설정) |
| `key_rule`, `fallback_to_source`, `include_deprecated` | 표 어댑터와 같다 |

## 명령 실행

- 명령을 먼저 공백으로 나누고(큰따옴표 안의 공백은 인자에 포함, 따옴표는 빠짐) 그다음 `{request}`를 요청 파일 경로로 바꾼다.
  그래서 요청 파일 경로에 공백이 있어도 따옴표를 붙일 필요가 없다. `--input={request}`처럼 인자 일부여도 된다.
- **셸을 거치지 않는다.** 파이프(`|`), `%변수%` 같은 셸 기능이 필요하면 `cmd /c ...`나 `powershell -File ...`을 직접 쓴다.
  `.bat`/`.cmd` 파일은 바로 실행된다. `.py`, `.ps1`은 `python ...`, `powershell -File ...`처럼 실행할 프로그램을 앞에 쓴다.
- **작업 폴더는 .tms 파일이 있는 폴더**다. `scripts/` 폴더를 .tms 옆에 두면 상대 경로로 부를 수 있다.
  프로그램 자리가 `scripts\run.bat`처럼 경로면 그 폴더 기준이고, `python`처럼 이름만 있으면 PATH 에서 찾는다.
- 환경 변수는 바꾸지 않는다. 터미널에서 직접 실행할 때와 같다. 콘솔 창은 띄우지 않는다.
- 남에게 받은 .tms 를 열면 그 사람이 적은 명령이 실행될 수 있다. 내보내기·가져오기 화면에 실행할 명령이 보이니 확인하고 누른다.

## 요청 (TMS → 프로그램)

요청 파일은 UTF-8 JSON 이다. 실행 기록 폴더(`%TEMP%\lingua-tms\<id>\request.json`)에 생긴다.

### 내보내기 (`action: "export"`)

```json
{
  "protocol": 1,
  "action": "export",
  "target": { "id": "0193…", "name": "제품 A" },
  "options": {},
  "locales": ["ko-KR", "en-US"],
  "path": "C:\\Users\\…\\Temp\\lingua-tms\\0193…\\제품 A.xlsx",
  "rows": [
    {
      "token_id": "0193…",
      "token_key": "home.title",
      "external_key": "STR_HOME_TITLE",
      "fixed": true,
      "values": { "ko-KR": "제목", "en-US": "Title" }
    }
  ]
}
```

- 프로그램은 **`path`에 파일을 쓴다.** 사용자가 고른 위치가 아니라 실행 기록 폴더 안이다(파일 이름은 같다).
  성공하면 TMS 가 사용자가 고른 위치로 옮긴다(있으면 덮어씀). 실패하면 기존 파일은 그대로다.
- `values`에는 값이 있는 언어만 들어 있다. "원문으로 채우기"가 켜져 있으면 이미 원문으로 채워져 있다.
- `fixed`: 이미 고정된 제품 키면 true, 이번 내보내기로 처음 고정될 키면 false.
- **`external_key`를 바꾸지 않는다.** 역가져오기가 이 키로 토큰을 찾는다.

### 역가져오기 (`action: "import"`)

```json
{
  "protocol": 1,
  "action": "import",
  "target": { "id": "0193…", "name": "제품 A" },
  "options": {},
  "locales": ["ko-KR", "en-US"],
  "path": "D:\\product\\strings.xlsx",
  "response_path": "C:\\Users\\…\\Temp\\lingua-tms\\0193…\\response.json"
}
```

- `path`: 사용자가 고른 제품 파일. 프로그램은 이 파일을 읽어 `response_path`에 응답을 쓴다.

## 응답 (역가져오기만)

`response_path`에 UTF-8 JSON(BOM 있어도 됨)으로 쓴다.

```json
{ "rows": [ { "external_key": "STR_HOME_TITLE", "values": { "ko-KR": "제목", "en-US": "Title" } } ] }
```

- `values`의 언어는 요청의 `locales` 안에 있어야 한다. 없는 언어가 있으면 오류로 본다.
- 빈 문자열이나 `null`은 빈 칸이다. 가져오기 옵션에 따라 건너뛰거나 지운다.
- 찾지 못한 제품 키, 원문으로 채워 넣었던 값 걸러내기는 TMS 가 한다.

## 성공과 실패

- **종료 코드 0 이 성공**이다. 내보내기는 `path`에 파일이 있어야, 역가져오기는 응답 파일을 읽을 수 있어야 성공이다.
- 실패하면 제품 키를 고정하지 않고(변경 묶음도 남지 않음), 오류 띠에 아래를 보여 준다.
  - `명령 실패 (종료 코드 N)` — 숫자의 뜻은 TMS 가 해석하지 않는다. Windows 예외로 끝나 음수면 16진수도 같이 보인다
    (예: `-1073741819 (0xC0000005)`).
  - 오류 출력(stderr) 끝 20줄. 비어 있으면 일반 출력(stdout) 끝 20줄.
  - 시간 초과는 `명령이 N초 안에 끝나지 않아 중단함`, 실행 자체를 못 하면 `'python'을(를) 실행하지 못함: …`.
  - 실행 기록 폴더 경로. 실패하면 지우지 않고 `request.json`, `stdout.txt`, `stderr.txt`를 남긴다.
    같은 요청으로 다시 돌려 볼 수 있다: `python scripts\product_a.py export <그 폴더>\request.json`
- 출력은 **UTF-8 로 읽는다.** UTF-8 이 아닌 글자는 화면에서 �로 보인다(원문은 stdout.txt, stderr.txt 에 그대로 있다).
  - 파이썬: `sys.stderr.reconfigure(encoding="utf-8")`
  - PowerShell: `[Console]::OutputEncoding = [Text.Encoding]::UTF8` 또는 바이트로 직접 쓰기
- 성공하면 실행 기록 폴더는 지운다.

## 프로그램을 만들 때

- 요청·응답 파일은 인코딩을 명시해서 연다: 파이썬 `open(path, encoding="utf-8")`, PowerShell `Get-Content -Raw -Encoding UTF8`.
- 로그는 stdout, stderr 어디에 찍어도 된다(TMS 는 출력을 해석하지 않고 실패할 때 보여 주기만 한다).
- Windows PowerShell 5.1 에서 한글이 든 `.ps1`은 UTF-8(BOM 있음)으로 저장한다. BOM 이 없으면 시스템 코드 페이지로 읽는다.
- 요청의 `protocol`이 모르는 값이면 0 이 아닌 코드로 끝내는 것이 안전하다. 형식이 바뀌면 TMS 가 이 숫자를 올린다.
