//! 외부 명령 어댑터(`command`): 제품 형식 파일 쓰기·읽기를 사용자가 지정한 프로그램(스크립트, exe 등)에 맡긴다.
//!
//! 주고받는 방식(프로토콜 1)은 `docs/command-adapter.md`에 있다. 요약:
//! - TMS 가 요청 JSON(UTF-8)을 실행 기록 폴더(임시 폴더 아래)에 쓰고, 설정의 명령에서 `{request}`를 그 경로로
//!   바꿔 실행한다. 셸을 거치지 않고, 작업 폴더는 .tms 파일이 있는 폴더, 환경 변수는 그대로다.
//! - 내보내기: 프로그램이 요청의 `path`(실행 기록 폴더 안)에 파일을 쓴다. 성공하면 TMS 가 사용자가 고른 위치로 옮긴다.
//! - 역가져오기: 프로그램이 요청의 `response_path`에 `{"rows": [...]}`를 쓴다.
//! - 종료 코드 0 이 성공이다. 그 밖에는 종료 코드와 출력 끝부분(UTF-8 로 읽음)을 오류로 돌려주고,
//!   실행 기록 폴더(request.json, stdout.txt, stderr.txt)를 지우지 않고 남긴다. 종료 코드의 뜻은 해석하지 않는다.
//!
//! TMS 는 스크립트를 저장하지 않고 파이썬 같은 런타임도 넣지 않는다. 명령이 가리키는 프로그램은 사용자가 준비한다.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{AdapterContext, AdapterOptions, ExportRow, ExternalRow, FormatAdapter, KeyRule};
use crate::ids::new_id;
use crate::{Error, Result};

/// 요청 JSON 의 `protocol` 값. 요청·응답 형식을 바꾸면 올린다.
pub const PROTOCOL_VERSION: u32 = 1;
/// 명령에서 요청 파일 경로로 바뀌는 자리.
pub const REQUEST_PLACEHOLDER: &str = "{request}";
/// 오류 메시지에 넣는 출력 끝부분 줄 수.
const TAIL_LINES: usize = 20;

pub struct CommandAdapter;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandConfig {
    /// 내보내기 명령. 예: `python scripts/product_a.py export {request}`
    #[serde(default)]
    pub export_command: String,
    /// 역가져오기 명령. 비어 있으면 이 대상은 역가져오기를 지원하지 않는다.
    #[serde(default)]
    pub import_command: String,
    /// 내보낼 언어. 이 순서대로 요청에 넣는다.
    #[serde(default)]
    pub locales: Vec<String>,
    /// 제품 파일 확장자(점 없이). 파일 대화상자에 쓴다.
    #[serde(default = "default_extension")]
    pub file_extension: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// 해석하지 않고 요청의 `options`로 그대로 넘긴다(제품별 설정).
    #[serde(default = "default_options")]
    pub options: Value,
    #[serde(default)]
    pub key_rule: KeyRule,
    #[serde(default)]
    pub fallback_to_source: bool,
    #[serde(default)]
    pub include_deprecated: bool,
}

fn default_extension() -> String {
    "xlsx".into()
}

fn default_timeout() -> u64 {
    60
}

fn default_options() -> Value {
    json!({})
}

impl CommandAdapter {
    fn config(config: &Value) -> Result<CommandConfig> {
        serde_json::from_value(config.clone()).map_err(|e| Error::Invalid(format!("명령 설정 오류: {e}")))
    }
}

/// 명령 한 줄을 프로그램과 인자로 나눈다. 공백으로 나누고, 큰따옴표로 감싼 부분의 공백은 인자에 넣는다
/// (따옴표 자체는 빠진다). 역슬래시는 그대로 둔다(Windows 경로).
pub fn split_command(line: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    // `""`처럼 빈 인자도 인자로 친다
    let mut started = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err(Error::Invalid("닫히지 않은 큰따옴표가 있음".into()));
    }
    if started {
        args.push(current);
    }
    if args.is_empty() {
        return Err(Error::Invalid("명령이 비어 있음".into()));
    }
    Ok(args)
}

fn check_command(label: &str, line: &str) -> Result<()> {
    if line.trim().is_empty() {
        return Err(Error::Invalid(format!("{label}이 비어 있음")));
    }
    if !line.contains(REQUEST_PLACEHOLDER) {
        return Err(Error::Invalid(format!(
            "{label}에 {REQUEST_PLACEHOLDER} 자리가 없음(요청 파일 경로가 들어갈 자리)"
        )));
    }
    split_command(line).map_err(|e| Error::Invalid(format!("{label}: {e}")))?;
    Ok(())
}

/// 실행할 프로그램과 인자. `{request}`를 요청 파일 경로로 바꾸고(나눈 뒤에 바꾸므로 경로에 공백이 있어도 된다),
/// 프로그램이 상대 경로(`scripts\run.bat` 등)면 작업 폴더 기준으로 바꾼다. `python` 처럼 경로 구분자가 없으면 PATH 에서 찾는다.
fn command_line(line: &str, request: &Path, work_dir: Option<&Path>) -> Result<Vec<String>> {
    let request = request.to_string_lossy();
    let mut argv: Vec<String> = split_command(line)?
        .into_iter()
        .map(|a| a.replace(REQUEST_PLACEHOLDER, &request))
        .collect();
    let program = Path::new(&argv[0]);
    if let Some(dir) = work_dir {
        if program.is_relative() && argv[0].contains(['/', '\\']) {
            argv[0] = dir.join(program).to_string_lossy().into_owned();
        }
    }
    Ok(argv)
}

impl FormatAdapter for CommandAdapter {
    fn id(&self) -> &'static str {
        "command"
    }

    fn description(&self) -> &'static str {
        "외부 명령: 요청 JSON 을 받아 지정한 프로그램이 제품 파일을 쓰고 읽음"
    }

    fn check_config(&self, config: &Value, locales: &HashSet<String>) -> Result<()> {
        let c = Self::config(config)?;
        check_command("내보내기 명령", &c.export_command)?;
        if !c.import_command.trim().is_empty() {
            check_command("역가져오기 명령", &c.import_command)?;
        }
        if c.locales.is_empty() {
            return Err(Error::Invalid("내보낼 언어가 하나도 없음".into()));
        }
        let mut seen = HashSet::new();
        for l in &c.locales {
            if !locales.contains(l) {
                return Err(Error::Invalid(format!("프로젝트에 없는 언어: {l}")));
            }
            if !seen.insert(l) {
                return Err(Error::Invalid(format!("언어가 겹침: {l}")));
            }
        }
        if c.file_extension.is_empty() || !c.file_extension.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            return Err(Error::Invalid("파일 확장자는 점 없이 영문·숫자로 적음(예: xlsx)".into()));
        }
        if !(1..=3600).contains(&c.timeout_secs) {
            return Err(Error::Invalid("시간 제한은 1~3600초".into()));
        }
        if !c.options.is_object() {
            return Err(Error::Invalid("options 는 JSON 객체여야 함".into()));
        }
        Ok(())
    }

    fn options(&self, config: &Value) -> Result<AdapterOptions> {
        let c = Self::config(config)?;
        Ok(AdapterOptions {
            locales: c.locales,
            key_rule: c.key_rule,
            fallback_to_source: c.fallback_to_source,
            include_deprecated: c.include_deprecated,
        })
    }

    fn write(&self, ctx: &AdapterContext, rows: &[ExportRow], path: &Path) -> Result<()> {
        let c = Self::config(&ctx.target.config)?;
        let file_name = path
            .file_name()
            .ok_or_else(|| Error::Invalid(format!("파일 이름이 없는 경로: {}", path.display())))?;
        let mut run = Run::new()?;
        let out = run.dir.join(file_name);
        let request = json!({
            "protocol": PROTOCOL_VERSION,
            "action": "export",
            "target": { "id": ctx.target.id, "name": ctx.target.name },
            "options": c.options,
            "locales": c.locales,
            "path": out,
            "rows": rows,
        });
        run.execute(&c.export_command, &request, ctx, c.timeout_secs)?;
        if !out.is_file() {
            return Err(run.fail("명령은 성공(종료 코드 0)했지만 요청의 path 에 파일을 만들지 않음"));
        }
        if let Err(e) = move_file(&out, path) {
            return Err(run.fail(format!("만든 파일을 {}(으)로 옮기지 못함: {e}", path.display())));
        }
        Ok(())
    }

    fn read(&self, ctx: &AdapterContext, path: &Path) -> Result<Vec<ExternalRow>> {
        let c = Self::config(&ctx.target.config)?;
        if c.import_command.trim().is_empty() {
            return Err(Error::Invalid(
                "이 대상은 역가져오기 명령이 없어 제품 형식 파일을 읽을 수 없음".into(),
            ));
        }
        let mut run = Run::new()?;
        let response = run.dir.join("response.json");
        let request = json!({
            "protocol": PROTOCOL_VERSION,
            "action": "import",
            "target": { "id": ctx.target.id, "name": ctx.target.name },
            "options": c.options,
            "locales": c.locales,
            "path": path,
            "response_path": response,
        });
        run.execute(&c.import_command, &request, ctx, c.timeout_secs)?;
        let raw = match fs::read(&response) {
            Ok(b) => b,
            Err(e) => return Err(run.fail(format!("명령은 성공(종료 코드 0)했지만 응답 파일을 읽지 못함: {e}"))),
        };
        // Windows PowerShell 5.1 등이 붙이는 BOM 은 받아 준다
        let body = raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&raw);
        let parsed: ImportResponse = match serde_json::from_slice(body) {
            Ok(r) => r,
            Err(e) => return Err(run.fail(format!("응답 JSON 오류: {e}"))),
        };
        let allowed: HashSet<&str> = c.locales.iter().map(String::as_str).collect();
        let mut out = Vec::with_capacity(parsed.rows.len());
        for row in parsed.rows {
            let key = row.external_key.trim().to_string();
            if key.is_empty() {
                return Err(run.fail("응답에 제품 키(external_key)가 빈 행이 있음"));
            }
            let mut values = Vec::with_capacity(row.values.len());
            for (locale, value) in row.values {
                if !allowed.contains(locale.as_str()) {
                    return Err(run.fail(format!("응답에 대상에 없는 언어가 있음: {locale}")));
                }
                values.push((locale, value.unwrap_or_default()));
            }
            out.push(ExternalRow { external_key: key, values });
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
struct ImportResponse {
    rows: Vec<ResponseRow>,
}

#[derive(Deserialize)]
struct ResponseRow {
    external_key: String,
    /// 언어 → 값. null 은 빈 값으로 본다.
    #[serde(default)]
    values: BTreeMap<String, Option<String>>,
}

/// 같은 드라이브면 이름 바꾸기, 아니면 복사. 대상 파일이 있으면 덮어쓴다.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)?;
    fs::remove_file(from).ok();
    Ok(())
}

fn exit_code(status: ExitStatus) -> String {
    match status.code() {
        // Windows 예외로 끝난 경우(0xC0000005 등)는 16진수도 같이 보여 준다
        Some(c) if c < 0 => format!("{c} (0x{:08X})", c as u32),
        Some(c) => c.to_string(),
        None => "없음".into(),
    }
}

/// 명령 한 번의 실행 기록 폴더(임시 폴더 아래). 성공하면 지우고, 실패하면 남겨 경로를 오류에 넣는다.
struct Run {
    dir: PathBuf,
    keep: bool,
}

impl Run {
    fn new() -> Result<Self> {
        let dir = std::env::temp_dir().join("lingua-tms").join(new_id());
        fs::create_dir_all(&dir)?;
        Ok(Self { dir, keep: false })
    }

    /// 실패 오류를 만든다. 실행 기록 폴더는 남긴다.
    fn fail(&mut self, message: impl Into<String>) -> Error {
        self.keep = true;
        Error::Command(format!(
            "{}\n실행 기록: {} (request.json, stdout.txt, stderr.txt)",
            message.into(),
            self.dir.display()
        ))
    }

    fn execute(&mut self, line: &str, request: &Value, ctx: &AdapterContext, timeout_secs: u64) -> Result<()> {
        let request_path = self.dir.join("request.json");
        fs::write(&request_path, serde_json::to_vec_pretty(request)?)?;
        let argv = command_line(line, &request_path, ctx.project_dir.as_deref())?;
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(File::create(self.dir.join("stdout.txt"))?)
            .stderr(File::create(self.dir.join("stderr.txt"))?);
        if let Some(dir) = &ctx.project_dir {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // 화면 앱에서 실행할 때 콘솔 창이 뜨지 않게 한다
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Err(self.fail(format!("'{}'을(를) 실행하지 못함: {e}", argv[0]))),
        };
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().ok();
                child.wait().ok();
                return Err(self.fail(format!(
                    "명령이 {timeout_secs}초 안에 끝나지 않아 중단함{}",
                    self.output_tail()
                )));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        if !status.success() {
            return Err(self.fail(format!("명령 실패 (종료 코드 {}){}", exit_code(status), self.output_tail())));
        }
        Ok(())
    }

    /// 오류 출력(비어 있으면 일반 출력)의 끝부분. UTF-8 로 읽고, UTF-8 이 아닌 글자는 �로 바뀐다.
    fn output_tail(&self) -> String {
        for (file, label) in [("stderr.txt", "오류 출력"), ("stdout.txt", "출력")] {
            let bytes = fs::read(self.dir.join(file)).unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes);
            let text = text.trim_end();
            if text.trim().is_empty() {
                continue;
            }
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(TAIL_LINES);
            let skipped = if start > 0 { format!("(앞 {start}줄 생략)\n") } else { String::new() };
            return format!("\n── {label} 끝부분 ──\n{skipped}{}", lines[start..].join("\n"));
        }
        String::new()
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if !self.keep {
            fs::remove_dir_all(&self.dir).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{export_to_file, plan_export, read_target_file, save_target, ExportTargetInput};
    use crate::history::apply;
    use crate::import::{plan_import, ImportOptions};
    use crate::model::{CellChange, CellKey, CellValue, ChangesetKind, ChangesetMeta};
    use crate::validate::Registry;
    use crate::{db, tokens};
    use rusqlite::Connection;

    #[test]
    fn splits_command_lines() {
        assert_eq!(
            split_command(r#"python "C:\My Tools\a.py" export {request}"#).unwrap(),
            vec!["python", r"C:\My Tools\a.py", "export", "{request}"]
        );
        assert_eq!(split_command(r#"a "" --x="b c""#).unwrap(), vec!["a", "", "--x=b c"]);
        assert!(split_command("  ").is_err());
        assert!(split_command(r#"python "a.py"#).is_err());

        let argv = command_line("scripts/run.bat --in={request}", Path::new(r"C:\t\request.json"), Some(Path::new(r"C:\p"))).unwrap();
        assert_eq!(argv, vec![r"C:\p\scripts/run.bat", r"--in=C:\t\request.json"]);
        let argv = command_line("python a.py {request}", Path::new("r.json"), Some(Path::new(r"C:\p"))).unwrap();
        assert_eq!(argv[0], "python");
    }

    /// 파일로 된 프로젝트(작업 폴더 = 프로젝트 폴더). 토큰 두 개, common.ok 의 en-US 는 비어 있다.
    fn project() -> (PathBuf, Connection) {
        let dir = std::env::temp_dir().join(format!("tms-command-{}", new_id()));
        fs::create_dir_all(dir.join("scripts")).unwrap();
        let mut conn = db::open(dir.join("project.tms")).unwrap();
        tokens::add_locale(&conn, "ko-KR", "한국어", true).unwrap();
        tokens::add_locale(&conn, "en-US", "English", false).unwrap();
        let a = tokens::create_token(&mut conn, "t", "home.title", None).unwrap();
        let b = tokens::create_token(&mut conn, "t", "common.ok", None).unwrap();
        let set = |t: &str, l: &str, v: &str| CellChange {
            key: CellKey::new(t, l),
            new: Some(CellValue::translated(v)),
        };
        apply(
            &mut conn,
            &ChangesetMeta::new("t", ChangesetKind::Edit),
            &[set(&a, "ko-KR", "제목"), set(&a, "en-US", "Title"), set(&b, "ko-KR", "확인")],
        )
        .unwrap();
        (dir, conn)
    }

    fn target(conn: &mut Connection, export_command: &str, import_command: &str, timeout_secs: u64) -> String {
        save_target(
            conn,
            "t",
            &ExportTargetInput {
                id: None,
                name: "제품 A".into(),
                adapter: "command".into(),
                config: json!({
                    "export_command": export_command,
                    "import_command": import_command,
                    "locales": ["ko-KR", "en-US"],
                    "file_extension": "csv",
                    "timeout_secs": timeout_secs,
                    "key_rule": { "prefix": "STR_", "case": "upper", "dot_replacement": "_" }
                }),
            },
        )
        .unwrap()
    }

    fn examples_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/command-adapter")
    }

    /// 내보내기 → 파일 확인 → 같은 파일 역가져오기 → 바뀐 칸 없음.
    fn round_trip(dir: &Path, conn: &mut Connection, export_command: &str, import_command: &str) {
        let target = target(conn, export_command, import_command, 60);
        // 공백과 한글이 든 경로
        let out = dir.join("내보내기 결과").join("제품 A.csv");
        fs::create_dir_all(out.parent().unwrap()).unwrap();
        let report = export_to_file(conn, "t", &Registry::builtin(), &target, &out, None).unwrap();
        assert_eq!((report.rows, report.new_keys), (2, 2));
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains("STR_HOME_TITLE") && text.contains("제목"), "{text}");
        assert_eq!(plan_export(conn, &target).unwrap().new_keys, 0);

        let table = read_target_file(conn, &target, &out).unwrap();
        assert_eq!(table.keys, vec!["common.ok", "home.title"]);
        assert!(table.unresolved.is_empty());
        assert_eq!(table.get("home.title", "ko-KR"), Some("제목"));
        let plan = plan_import(conn, &table, &ImportOptions::default()).unwrap();
        assert!(plan.changes.is_empty(), "{:?}", plan.changes);
    }

    fn powershell(script: &str, args: &str) -> String {
        format!("powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File {script} {args}")
    }

    #[cfg(windows)]
    #[test]
    fn round_trip_with_powershell_example() {
        let (dir, mut conn) = project();
        // 작업 폴더가 프로젝트 폴더인지 보려고 스크립트를 상대 경로로 부른다
        fs::copy(examples_dir().join("csv_adapter.ps1"), dir.join("scripts/csv_adapter.ps1")).unwrap();
        round_trip(
            &dir,
            &mut conn,
            &powershell("scripts/csv_adapter.ps1", "export {request}"),
            &powershell("scripts/csv_adapter.ps1", "import {request}"),
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn round_trip_with_python_example() {
        let python = Command::new("python").arg("--version").output();
        if !python.map(|o| o.status.success()).unwrap_or(false) {
            eprintln!("python 이 없어 건너뜀");
            return;
        }
        let (dir, mut conn) = project();
        let script = examples_dir().join("csv_adapter.py");
        let script = format!("\"{}\"", script.display());
        round_trip(
            &dir,
            &mut conn,
            &format!("python {script} export {{request}}"),
            &format!("python {script} import {{request}}"),
        );
        fs::remove_dir_all(dir).ok();
    }

    /// 실행 기록 폴더 경로(오류 메시지의 "실행 기록: ..." 줄).
    fn kept_dir(message: &str) -> PathBuf {
        let line = message.lines().find_map(|l| l.strip_prefix("실행 기록: ")).unwrap();
        PathBuf::from(line.split(" (").next().unwrap())
    }

    #[cfg(windows)]
    #[test]
    fn failures_report_exit_code_and_fix_nothing() {
        let (dir, mut conn) = project();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command_fixture.ps1");
        let fixture = format!("\"{}\"", fixture.display());
        let reg = Registry::builtin();
        let out = dir.join("out.csv");
        fs::write(&out, "기존 파일").unwrap();

        // 0 이 아닌 종료 코드: 코드와 오류 출력, 실행 기록 폴더를 알려 주고 키는 고정하지 않는다
        let t = target(&mut conn, &powershell(&fixture, "fail {request}"), "", 60);
        let Err(Error::Command(m)) = export_to_file(&mut conn, "t", &reg, &t, &out, None) else {
            panic!("명령 오류여야 함");
        };
        assert!(m.contains("종료 코드 3") && m.contains("제품 형식이 다름"), "{m}");
        let kept = kept_dir(&m);
        assert!(kept.join("request.json").is_file() && kept.join("stderr.txt").is_file());
        fs::remove_dir_all(kept).ok();
        assert_eq!(plan_export(&conn, &t).unwrap().new_keys, 2);
        assert_eq!(fs::read_to_string(&out).unwrap(), "기존 파일");

        // 종료 코드 0 인데 파일을 만들지 않음
        let t = target(&mut conn, &powershell(&fixture, "nofile {request}"), "", 60);
        let Err(Error::Command(m)) = export_to_file(&mut conn, "t", &reg, &t, &out, None) else {
            panic!("명령 오류여야 함");
        };
        assert!(m.contains("파일을 만들지 않음"), "{m}");
        fs::remove_dir_all(kept_dir(&m)).ok();

        // 시간 초과
        let t = target(&mut conn, &powershell(&fixture, "sleep {request}"), "", 1);
        let Err(Error::Command(m)) = export_to_file(&mut conn, "t", &reg, &t, &out, None) else {
            panic!("명령 오류여야 함");
        };
        assert!(m.contains("1초 안에 끝나지 않아"), "{m}");
        fs::remove_dir_all(kept_dir(&m)).ok();

        // 역가져오기 명령이 없는 대상
        assert!(read_target_file(&conn, &t, &out).is_err());
        assert_eq!(fs::read_to_string(&out).unwrap(), "기존 파일");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_program_and_bad_config() {
        let (dir, mut conn) = project();
        let t = target(&mut conn, "tms-no-such-program {request}", "", 60);
        let out = dir.join("out.csv");
        let Err(Error::Command(m)) = export_to_file(&mut conn, "t", &Registry::builtin(), &t, &out, None) else {
            panic!("명령 오류여야 함");
        };
        assert!(m.contains("실행하지 못함"), "{m}");
        fs::remove_dir_all(kept_dir(&m)).ok();

        let mut bad = |config: Value| {
            save_target(
                &mut conn,
                "t",
                &ExportTargetInput {
                    id: None,
                    name: "x".into(),
                    adapter: "command".into(),
                    config,
                },
            )
            .is_err()
        };
        assert!(bad(json!({ "export_command": "python a.py", "locales": ["ko-KR"] })));
        assert!(bad(json!({ "export_command": "python \"a.py {request}", "locales": ["ko-KR"] })));
        assert!(bad(json!({ "export_command": "python a.py {request}", "locales": ["fr-FR"] })));
        assert!(bad(json!({ "export_command": "python a.py {request}", "locales": ["ko-KR"], "file_extension": ".csv" })));
        assert!(bad(json!({ "export_command": "a {request}", "import_command": "b", "locales": ["ko-KR"] })));
        assert!(!bad(json!({ "export_command": "a {request}", "locales": ["ko-KR"] })));
        drop(conn);
        fs::remove_dir_all(dir).ok();
    }
}
