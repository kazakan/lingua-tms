# Lingua TMS — 작업 안내 (Claude Code용)

엑셀로 관리하던 다국어 UI 문구 DB(토큰명 열 + 언어 열)를 데스크톱 앱으로 옮기는 프로젝트다.
핵심 기능은 문구별 검증이다. "이 폰트로 넘치면 안 됨", "박스 폭 W에서 자동 줄바꿈하면 정확히 2줄"
같은 규칙을 Figma를 열지 않고 확인한다.

- 사용자와는 **한국어**로 대화한다. 코드 주석도 한국어가 기본이다.
- **커밋 메시지는 영문**으로 쓴다(제목·본문 모두).
- 이 폴더는 claude.ai 채팅(설계 논의)과 Claude Code(빌드·디버깅)가 번갈아 작업한다.
  채팅에서 결정이 바뀌면 이 파일의 "결정 사항"을 고친다. 코드와 이 파일이 다르면 사용자에게 묻는다.
- 전체 요구사항 문서: https://claude.ai/artifact/Ce9icjLCmXUTuwnasDYpaC (탭: 요구사항 정의서, MVP 구현 계획).
  로그인이 필요해 Claude Code에서는 못 읽는다. 필요한 내용은 아래에 요약해 두었다.

## 폴더 구조

```
Cargo.toml            Rust 워크스페이스
crates/core           tms-core: SQLite 저장소, 변경 이력, diff, 검증기. 화면과 무관, 로직은 전부 여기
  migrations/         스키마(SQL). 추가만 한다
  src/db.rs           프로젝트 파일 열기, 연결 설정, 마이그레이션
  src/history.rs      문구값 쓰기의 유일한 경로(apply), 과거 시점, diff, 되돌리기, 롤백, 릴리스
  src/tokens.rs       토큰·언어. 토큰명 변경 이력
  src/grid.rs         편집 화면용 조회(행 = 토큰, 열 = 언어, outdated 표시)
  src/validate/       검증기 레지스트리, 내장 검증기 9종, 제약 해석
crates/measure        tms-measure: Measurer 트레이트 + 엔진 DLL 바인딩(libloading)
crates/engine-stub    개발용 가짜 엔진 DLL. tms_measure.h 를 Rust로 구현
engine/include/tms_measure.h   엔진 팀과의 C 인터페이스 계약(ABI 1)
src-tauri/            Tauri 2 앱. 명령(IPC)만 두고 tms-core 를 호출한다
src/                  화면(React + TypeScript + Vite). api.ts 가 Rust 명령 래퍼
```

## 빌드와 실행 (Windows)

준비물: Rust stable(MSVC, `rustup default stable-x86_64-pc-windows-msvc`), Visual Studio Build Tools의
"C++를 사용한 데스크톱 개발"(rusqlite 가 SQLite C 코드를 컴파일함), Node.js 22 이상,
WebView2(Windows 10/11 기본 포함).

```powershell
npm install
npm run tauri dev                 # 앱 실행(Vite 1420 포트 + Rust 빌드)
cargo test --workspace            # Rust 테스트 전체(가짜 엔진 DLL FFI 테스트 포함)
cargo clippy --workspace --all-targets
npm run build                     # 화면 타입 검사 + 빌드
npm run tauri build               # 설치 파일(NSIS) → target/release/bundle/nsis
```

FFI 테스트(`crates/measure/tests/stub_engine.rs`)는 `tms_measure_stub.dll`을 `target/debug`와 `target/debug/deps`에서 찾는다.
- `cargo test --workspace`도 스텁 DLL을 빌드하지만 위치가 cargo 버전마다 다르다.
  - 로컬(cargo 1.99)에서는 `deps`에 생긴다.
  - GitHub 러너의 cargo는 `target/debug/build/…/out` 쪽에 둬서 테스트가 못 찾았다.
  - 확실히 돌리려면 먼저 `cargo build -p tms-engine-stub`을 실행한다. 이러면 `target/debug`에 생긴다. CI가 이렇게 한다.
- 다른 DLL을 시험하려면 `TMS_ENGINE_DLL` 환경 변수에 경로를 준다.
- 엔진 DLL은 경우에 따라 없을 수 있다. DLL이 없으면 FFI 테스트는 실패하지 않고 건너뛴다(CI도 같음).
  `-- --nocapture`로 돌리면 찾은 DLL 경로나 건너뛴 이유가 출력된다.

## 버전과 릴리스

- 저장소: https://github.com/kazakan/lingua-tms (public, 기본 브랜치 `main`)
- **Semantic Versioning**을 따른다. MVP 단계에서는 major를 항상 0으로 둔다(`0.MINOR.PATCH`).
- 버전은 세 곳을 같이 고친다. lock 파일은 `cargo build`, `npm install`이 맞춰 준다.
  - `Cargo.toml`의 `[workspace.package] version`
  - `package.json`의 `version`
  - `src-tauri/tauri.conf.json`의 `version`
- 릴리스 방법: 버전을 올린 커밋을 main에 넣고 `vX.Y.Z` 태그를 push한다.
  - `.github/workflows/release.yml`이 테스트 후 NSIS 설치 파일을 빌드해 GitHub Release에 올린다.
  - 태그와 위 세 파일의 버전이 다르면 실패한다.
- CI(`.github/workflows/ci.yml`): main push와 PR마다 Windows에서 `npm run build`, clippy(`-D warnings`), `cargo test --workspace`.
- 설치 파일은 코드 서명이 없다. 처음 실행할 때 SmartScreen 경고가 뜬다.

## 현재 상태 (2026-11-24)

- 버전 0.0.1. Windows 11에서 확인한 것(TODO 1 완료):
  - `cargo test --workspace` 통과(core 19, engine-stub 2, 스텁 DLL FFI 1)
  - clippy(`-D warnings`) 경고 없음, `npm run build` 성공
  - `npm run tauri dev`로 창을 띄워 새 프로젝트 → 언어 추가 → 토큰 추가 → 셀 편집 → 이력 확인까지 동작
  - 원문이 나중에 바뀐 셀의 노란색 표시도 확인
- git 저장소와 GitHub(public), CI·릴리스 워크플로가 있다(위 "버전과 릴리스").
- 화면은 뼈대 수준이다.
  - 프로젝트 새로 만들기·열기(.tms 파일), 언어 추가, 토큰 추가, 토큰명 변경
  - 문구 표(Glide): 셀 편집, 여러 셀 복사·붙여넣기, Delete 로 지우기, 검색·필터
  - 원문이 더 최근에 바뀐 셀 노란색, 미번역 회색, 선택한 셀의 이력 보기
- 검증기는 코어에 구현·테스트만 되어 있고 화면과 연결되지 않았다. `list_validators` 명령만 있다.

## 지켜야 할 규칙

1. **문구값(`translation`)은 `history::apply`로만 쓴다.** 직접 INSERT/UPDATE/DELETE 하면 트리거가 거부한다.
   한 번의 사용자 동작(셀 편집, 엑셀 가져오기 한 번)이 변경 묶음(changeset) 하나다.
2. 이력 테이블(`translation_rev`, `entity_rev`, `token_key_history`)은 추가만 한다.
   롤백·되돌리기도 과거 값을 새 변경 묶음으로 쓴다. 이력 행을 지우거나 고치지 않는다.
3. 마이그레이션은 추가만 한다.
   - 기존 `0001_init.sql`은 고치지 않는다.
   - 새 파일 `000N_*.sql`을 만들고 `db.rs`의 `MIGRATIONS`에 등록한다.
   - 테이블은 STRICT로 만든다.
4. DB 연결은 `db::open`으로만 연다. `foreign_keys`는 연결마다 켜야 하는데 이 함수가 처리한다.
5. 토큰의 `id`(UUIDv7)는 바뀌지 않고 `key`(토큰명)는 바뀔 수 있다.
   - 이전 이름은 `token_key_history`에 남는다.
   - 다른 토큰이 이전 이름을 다시 쓸 수 없다. 트리거가 막는다.
   - 엑셀 재가져오기에서 이름을 찾을 때는 `tokens::find_token_by_key`를 쓴다. 이전 이름까지 찾는다.
6. 제품 키(`token_external_key`)는 한 번 내보내면 고정이다. 규칙(접두어 등)은 새 토큰의 키를 제안할 때만 쓴다.
7. **렌더링 측정(폭, 줄 수, 누락 글자)은 엔진 DLL(`Measurer`)로만 한다.**
   TMS 안에서 폭을 따로 계산하면 기기 결과와 달라진다. 단, 테스트용 가짜 측정기는 예외다.
8. `engine/include/tms_measure.h`는 엔진 팀과의 계약이다. 구조체나 함수를 바꾸면 아래를 같이 고친다.
   - `TMS_MEASURE_ABI_VERSION`
   - `crates/measure/src/ffi.rs`의 `ABI_VERSION`과 repr(C) 구조체
   - `crates/engine-stub`
9. `src-tauri`는 얇게 유지한다. 로직은 `tms-core`에 두고 거기서 테스트한다.
   - 명령을 추가하면 `generate_handler!`와 `src/api.ts`를 같이 고친다.
   - 인자 이름은 JS에서 camelCase, Rust에서 snake_case다. Tauri가 자동 변환한다.
10. 새 검증기는 `Validator` 트레이트를 구현하고 `Registry::builtin`에 등록한다.
    - `params_schema`(JSON Schema)로 제약 입력 폼이 만들어진다.
    - 검증기 id·파라미터 형식을 바꾸면 `version()`을 올린다.

## 결정 사항 (요약)

- **형태**: 1차는 단일 사용자 Windows 데스크톱 앱이다. 설치 파일 하나, 서버 없음.
  - 프로젝트 하나 = SQLite 파일 하나(.tms).
  - 동시 편집, 계정, 외부 API는 이후 단계다.
- **스택**:
  - Rust + Tauri 2, 화면은 React. 문구 표는 Glide Data Grid.
  - 저장은 rusqlite(bundled), 토큰 id는 UUIDv7.
  - 엑셀은 calamine(읽기)과 rust_xlsxwriter(쓰기).
- **규모**: 토큰 약 1,600개, 언어 7개(셀 약 1만 1천), 연간 10~20개 증가. 언어 하나가 통으로 추가될 수 있다.
  - 검증은 캐시 없이 전체 재검증으로 시작한다.
- **이력**: Dolt 대신 SQLite에 직접 구현한다.
  - `changeset.id`가 전역 버전 번호다.
  - `translation`은 현재 값 캐시이고, `version`은 그 셀을 마지막으로 바꾼 changeset이다.
  - `translation_rev`는 셀 변경 1건이 1행이다.
  - `release`는 changeset에 붙인 이름표다.
  - 과거 시점은 rev에서 ROW_NUMBER로 계산하고, diff는 rev 구간의 처음 old와 마지막 new로 구한다.
  - Dolt 재검토 조건: 브랜치형 번역 작업이 필요해질 때, 이력 대상이 많아져 유지가 부담될 때, 동시 편집이 필요할 때.
- **제품 내보내기**: TMS 토큰명과 제품 키를 분리한다.
  - `export_target`(제품별 설정)과 `token_external_key`(제품 키 고정 저장)를 쓴다.
  - 엑셀 형식이 제품마다 달라 포맷 어댑터를 둔다. 기본은 설정형 표이고, 안 맞는 제품은 Rust 전용 어댑터를 쓴다.
  - 같은 어댑터가 역가져오기도 맡는다.
- **비교**: 모든 입력(DB 시점, 엑셀 파일)을 토큰 × 언어 → 값 집합으로 바꿔 diff 엔진 하나로 비교한다.
- **검증**: 검증기(규칙 유형) → 제약(검증기 + 파라미터 + 범위 + 언어 + 심각도) → 결과.
  - 범위는 전역 < 태그 < 토큰 순으로 구체적이고, 같은 범위에서는 언어를 지정한 쪽이 더 구체적이다.
  - 같은 검증기는 가장 구체적인 레벨만 적용하고, 다른 검증기는 누적한다.
  - 하위 레벨에서 enabled=0으로 두면 상위 제약을 끈다.
  - 심각도: error는 승인·내보내기 차단, warning은 사유를 적고 진행, info는 표시만 한다.
  - 내장 검증기 9종:
    - 텍스트: `text.required`, `text.length`, `text.placeholder_match`, `text.pattern`, `text.whitespace`
    - 렌더: `render.glyph_coverage`, `render.max_width`, `render.line_count`, `render.word_overflow`
- **측정**:
  - 대상은 주로 임베디드이고, 자체 엔진이 기기에서 .ttf를 직접 그린다.
  - 엔진의 글자 폭·줄바꿈 코드를 PC용 DLL로 빌드하고 `tms_measure.h`를 구현해 받는다.
  - 렌더 프로필(`render_profile`)은 엔진 버전, 크기, 자간, 줄 높이, 줄바꿈 방식, 언어별 폰트 폴백 순서를 가진다.
  - 엔진 버전별 DLL을 보관하고(`measure_engine`), 결과에 엔진 버전을 기록한다.
  - 엔진 DLL은 경우에 따라 없을 수 있다. DLL이 없어도 편집과 텍스트 검증은 동작해야 한다.
    렌더 검증기는 `RenderCtx` 없이 호출되면 `ValidateError::NeedsRender`를 돌려준다(죽지 않음).
  - 폰트 파일은 DB에 BLOB으로 넣는다(`font_asset`, sha256 중복 방지).
- **UI 규칙**: 행 = 토큰, 열 = 언어인 엑셀형 표는 화면용 뷰일 뿐이다. 저장은 토큰 × 언어 단위 행이다.

## 다음 작업 (순서대로)

1. ~~**Windows에서 확인**~~ 완료(0.0.1). 테스트·앱 실행 확인, git·GitHub·CI·릴리스 구성
2. ~~**표 교체**~~ 완료: Glide Data Grid(`6.0.4-alpha24` 고정, React 19 지원판은 베타뿐)
   - 가상 스크롤, 복사·붙여넣기, 검색·필터(미번역, 원문 변경됨; 검증 실패는 작업 5에서)
   - 편집·붙여넣기·지우기는 `set_cells` → `history::apply_edits` 로 changeset 하나. 값이 같은 셀은 건너뜀
3. ~~**엑셀 가져오기(calamine)**~~ 완료
   - `sheet.rs`(읽기·열 매핑) → `values::ValueTable` → `import.rs`(계획·적용)
   - 토큰 생성과 값 변경이 Import 변경 묶음 하나. 빈 칸은 기본으로 건너뜀(옵션으로 지우기)
   - 토큰명은 이전 이름까지 찾고, 이전 이름으로 찾은 행은 미리보기에 표시
4. ~~**내보내기와 포맷 어댑터**~~ 완료(`export.rs`)
   - `FormatAdapter` 트레이트, 기본 `table`(설정형 표). 새 Rust 어댑터는 `export::adapters()`에 추가
   - 제품 키: 내보낼 때 규칙으로 제안(겹치면 `_2`)하고 파일 쓰기와 한 트랜잭션으로 고정. 고정 전 직접 지정 가능
   - 역가져오기는 가져오기 탭의 "제품 형식 파일". 고정된 키로만 찾고, "원문으로 채우기"로 들어간 값은 건너뜀
5. ~~**제약과 검증 화면**~~ 완료(`validate/store.rs`, `validate/run.rs`)
   - 제약 저장·삭제(`entity_rev`), 토큰 태그(`tokens::set_tags`). 파라미터 폼은 react-jsonschema-form
   - `validate_all`: 캐시 없이 전체. 미번역 셀은 `checks_missing()` 검증기(`text.required`)만, 렌더 검증기는 건너뜀(개수만 셈)
   - 문구 표: 위반 셀 색, "검증 실패" 필터, 옆 패널에 위반 내용. 검증 결과에서 셀로 이동
   - 내보내기 게이트(`export::check_export`): error 는 차단, warning 은 사유를 받아 변경 묶음 메시지에 남김
   - 아직 없음: 상태(reviewed/approved) 바꾸는 승인 화면과 그 차단
6. **이력 화면**: 릴리스 만들기, 두 시점 diff, 되돌리기·롤백, 두 엑셀 파일 비교

**후순위** (2026-11-25 결정: 렌더 연결은 위 작업을 마친 뒤에 한다)

7. **렌더 연결**
   - `font_asset` 등록, `render_profile` 편집, `measure_engine`(DLL 경로)
   - 렌더 검증기를 `EngineDll`에 연결하고, 편집 중 폭·줄 수를 실시간으로 표시한다.
8. **실제 엔진 DLL** 받기, 골든 세트로 PC 결과와 기기 결과 일치 확인

## 미결 사항 (사용자·엔진 팀 확인 필요)

- 엔진 C 인터페이스 합의: 스레드 안전성, 폰트 폴백 순서, 누락 글자 보고, 말줄임 동작, 제품별 엔진 버전 목록
- 골든 세트 준비(기기 화면 캡처)
- 7개 언어 목록(복합 문자 포함 여부)
- 제품별 관리 엑셀·제품 엑셀 샘플, placeholder 문법
  - 현재 `text.placeholder_match`는 `{name}`, `%s`, `%1$s` 형식을 가정한다.
- 설치할 PC의 Windows 버전, 폐쇄망 여부(업데이트 방식에 영향)
