# Lingua TMS

다국어 UI 문구 관리 데스크톱 앱(단일 사용자). 문구별 검증(폰트 폭·줄 수 등)과 변경 이력·롤백을 지원한다.

- 앱: Rust + Tauri 2, 화면은 React
- 저장: 프로젝트 하나 = SQLite 파일 하나(.tms)
- 렌더 측정: 자체 엔진을 PC용 DLL로 빌드해 `engine/include/tms_measure.h` 인터페이스로 호출

```powershell
npm install
npm run tauri dev
cargo test --workspace
```

Windows 설치 파일은 [Releases](https://github.com/kazakan/lingua-tms/releases)에서 받는다. 버전은 Semantic Versioning을 따르며 MVP 동안은 `0.x.y`다.

설계 결정, 규칙, 다음 작업은 [CLAUDE.md](CLAUDE.md)에 있다.
